//! A match of Wordle: its rounds, the score, and what a key, click or
//! message does to them.
//!
//! Alone, a round is one player against the word. Against someone, both
//! guess the same word at once, each seeing only the colours of the other's
//! guesses until they are done themselves. Fewer guesses wins; the same
//! number is settled by who got there sooner, and failing both is a draw.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::protocol::{Bytes, Msg};
use super::ui::{Geometry, Key};
use super::words::{self, LEN, Mark, TRIES, Word};
use crate::games::{Ctx, Handled, Seat};

/// Tiles in a row, for timing them one after another.
#[expect(clippy::cast_possible_truncation, reason = "five")]
const TILES: u32 = LEN as u32;

/// Each tile of a guess starts turning over this long after the one before.
pub const STEP: Duration = Duration::from_millis(150);
/// How long one tile takes to turn over.
pub const TURN: Duration = Duration::from_millis(260);
/// A whole guess turning over, from the first tile to the last.
pub const REVEAL: Duration = STEP.saturating_mul(TILES - 1).saturating_add(TURN);
/// A solving row hops, one tile after another, once it has turned over.
pub const HOP_STEP: Duration = Duration::from_millis(80);
pub const HOP: Duration = Duration::from_millis(170);
pub const BOUNCE: Duration = HOP_STEP.saturating_mul(TILES - 1).saturating_add(HOP);
/// How long a guess that does not count shakes its row.
pub const SHAKE: Duration = Duration::from_millis(350);
/// How long a letter just typed stands out.
pub const POP: Duration = Duration::from_millis(110);
/// The pause between the round settling on screen and its card.
pub const CARD_DELAY: Duration = Duration::from_millis(300);
/// The card spells the word out a letter at a time.
pub const SPELL: Duration = Duration::from_millis(140);
pub const SPELT: Duration = SPELL.saturating_mul(TILES + 1);

/// Where a tile is in turning over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Face {
    /// Still showing just its letter.
    Up,
    /// Part way over, from 0 to 1: its letter face first, then its colour.
    Turning(f32),
    /// Showing its colour.
    Down,
}

/// One guess, marked.
#[derive(Clone, Debug)]
pub struct Guess {
    pub word: Word,
    pub marks: [Mark; LEN],
    /// How far into the round it came, as its player tells it.
    pub ms: u64,
    /// When it was made or arrived here, for turning its tiles over.
    pub at: Instant,
}

impl Guess {
    /// How far tile `i` has turned over by `now`.
    #[must_use]
    pub fn face(&self, i: usize, now: Instant) -> Face {
        let start = self.at + STEP * u32::try_from(i).unwrap_or(u32::MAX);
        if now < start {
            return Face::Up;
        }
        let t = (now - start).as_secs_f32() / TURN.as_secs_f32();
        if t >= 1.0 {
            Face::Down
        } else {
            Face::Turning(t)
        }
    }

    /// Whether tile `i` shows its colour by `now`: from half way over.
    #[must_use]
    pub fn shown(&self, i: usize, now: Instant) -> bool {
        match self.face(i, now) {
            Face::Up => false,
            Face::Turning(t) => t >= 0.5,
            Face::Down => true,
        }
    }

    /// Every tile has turned over.
    #[must_use]
    pub fn landed(&self, now: Instant) -> bool {
        now >= self.at + REVEAL
    }
}

/// One player's guesses at the word.
#[derive(Clone, Debug, Default)]
pub struct Board {
    pub guesses: Vec<Guess>,
}

impl Board {
    /// How many guesses it took, and how long, if the word was found.
    #[must_use]
    pub fn solved_in(&self) -> Option<(usize, u64)> {
        self.guesses
            .iter()
            .position(|g| words::solved(&g.marks))
            .map(|i| (i + 1, self.guesses[i].ms))
    }

    /// Found the word, or ran out of guesses.
    #[must_use]
    pub fn done(&self) -> bool {
        self.solved_in().is_some() || self.guesses.len() >= TRIES
    }

    /// The best that is known of `letter` by `now`, from the tiles that have
    /// turned over, so the keyboard does not give a guess away early.
    #[must_use]
    pub fn letter(&self, letter: u8, now: Instant) -> Option<Mark> {
        self.guesses
            .iter()
            .flat_map(|g| (0..LEN).map(move |i| (g, i)))
            .filter(|&(g, i)| g.word[i] == letter && g.shown(i, now))
            .map(|(g, i)| g.marks[i])
            .max()
    }

    /// The marks for `word` as this board's next guess, or why it cannot be.
    fn check(&self, word: Word, answer: Word, hard: bool) -> Result<[Mark; LEN], String> {
        if self.done() {
            return Err("no guesses left".into());
        }
        if !words::is_word(&word) {
            return Err(format!("{} is not in the word list", words::shout(&word)));
        }
        if hard {
            let earlier: Vec<_> = self.guesses.iter().map(|g| (g.word, g.marks)).collect();
            if let Some(why) = words::hard_miss(&earlier, &word) {
                return Err(why);
            }
        }
        Ok(words::score(&word, &answer))
    }
}

/// How a round went, for this player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Won,
    Lost,
    Drawn,
}

/// One word, and everyone's guesses at it.
#[derive(Clone, Debug)]
pub struct Round {
    /// Counting from 1.
    pub number: u32,
    pub answer: Word,
    pub hard: bool,
    pub started: Instant,
    pub me: Board,
    /// The opponent's guesses; `None` alone.
    pub them: Option<Board>,
    /// When the outcome was settled here, which it has been once it is in
    /// the score.
    pub settled_at: Option<Instant>,
}

impl Round {
    /// How the round went, once that is settled. Against someone it can be
    /// settled before both are done: solving in three beats anyone already
    /// three guesses in without it.
    #[must_use]
    pub fn outcome(&self) -> Option<Outcome> {
        let Some(them) = &self.them else {
            return self.me.done().then(|| {
                if self.me.solved_in().is_some() {
                    Outcome::Won
                } else {
                    Outcome::Lost
                }
            });
        };
        match (self.me.solved_in(), them.solved_in()) {
            (Some(mine), Some(theirs)) => Some(match mine.cmp(&theirs) {
                std::cmp::Ordering::Less => Outcome::Won,
                std::cmp::Ordering::Greater => Outcome::Lost,
                std::cmp::Ordering::Equal => Outcome::Drawn,
            }),
            (Some((n, _)), None) => (them.guesses.len() >= n).then_some(Outcome::Won),
            (None, Some((n, _))) => (self.me.guesses.len() >= n).then_some(Outcome::Lost),
            (None, None) => (self.me.done() && them.done()).then_some(Outcome::Drawn),
        }
    }

    #[must_use]
    pub fn over(&self) -> bool {
        self.outcome().is_some()
    }

    /// Whether the opponent's letters may be shown: only once ours can no
    /// longer change, or they would give the word away.
    #[must_use]
    pub fn show_theirs(&self) -> bool {
        self.me.done() || self.over()
    }
}

/// Agreeing on the next round with the opponent. See
/// [`protocol`](super::protocol) for how the word is drawn.
#[derive(Clone, Debug, Default)]
pub struct Setup {
    /// Our offer: its mode, and the secret we promised.
    pub mine: Option<(bool, Bytes)>,
    /// Their offer: its mode, and the hash of the secret they promised.
    pub theirs: Option<(bool, Bytes)>,
    /// Our secret is out, so our offer can no longer change.
    pub revealed: bool,
    /// Their secret, once it has come and kept its promise.
    pub their_secret: Option<Bytes>,
}

/// How the rounds have gone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Score {
    pub won: u32,
    pub lost: u32,
    pub drawn: u32,
    /// Words found in a row, alone.
    pub streak: u32,
    pub best_streak: u32,
    /// How many words were found in each number of guesses.
    pub spread: [u32; TRIES],
}

impl Score {
    #[must_use]
    pub fn played(&self) -> u32 {
        self.won + self.lost + self.drawn
    }
}

/// How loudly the status line should speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// The player has something to do.
    Go,
    /// Waiting on the other side.
    Wait,
    /// The opponent is asking something.
    Offer,
    /// Something went wrong.
    Warn,
    Win,
    Lose,
    Done,
}

pub struct App {
    pub seat: Seat,
    /// The mode the next round is offered in.
    pub hard: bool,
    /// The round being played, or the last one.
    pub round: Option<Round>,
    pub setup: Setup,
    pub score: Score,
    /// The guess being typed.
    pub typing: Vec<u8>,
    /// When a guess that did not count was turned away, to shake its row.
    pub shake: Option<Instant>,
    /// When the last letter was typed, for it to stand out a moment.
    pub typed_at: Option<Instant>,
    /// The card saying how the round went has been put away.
    pub card_hidden: bool,
    /// The opponent left, or was never reached: nothing more can happen
    /// against them.
    pub gone: bool,
    /// Overrides the animation clock, so a test can render an exact frame.
    pub clock: Option<Instant>,
}

impl App {
    /// A match at `seat`: alone when local, else against the opponent.
    #[must_use]
    pub fn new(seat: Seat) -> Self {
        Self {
            seat,
            hard: false,
            round: None,
            setup: Setup::default(),
            score: Score::default(),
            typing: Vec::new(),
            shake: None,
            typed_at: None,
            card_hidden: false,
            gone: false,
            clock: None,
        }
    }

    /// Playing without an opponent.
    #[must_use]
    pub fn alone(&self) -> bool {
        self.seat == Seat::Local
    }

    pub fn now(&self) -> Instant {
        self.clock.unwrap_or_else(Instant::now)
    }

    /// A round is on and not yet settled, with someone still to settle it.
    #[must_use]
    pub fn in_play(&self) -> bool {
        !self.gone && self.round.as_ref().is_some_and(|r| !r.over())
    }

    /// Whether keys are letters of a guess right now.
    #[must_use]
    pub fn guessing(&self) -> bool {
        self.in_play() && self.round.as_ref().is_some_and(|r| !r.me.done())
    }

    /// What enter does between rounds.
    #[must_use]
    pub fn next_label(&self) -> &'static str {
        match (&self.round, self.alone()) {
            (None, true) => "start",
            (None, false) => "ready",
            (Some(_), true) => "next word",
            (Some(_), false) => "rematch",
        }
    }

    #[must_use]
    pub fn is_animating(&self) -> bool {
        let now = self.now();
        let shaking = self.shake.is_some_and(|at| now < at + SHAKE);
        let turning = self.round.as_ref().is_some_and(|r| {
            let last = |b: &Board| b.guesses.last().is_some_and(|g| !g.landed(now));
            last(&r.me) || r.them.as_ref().is_some_and(last)
        });
        let popping = self.typed_at.is_some_and(|at| now < at + POP);
        let card = self.card_start().is_some_and(|at| now < at + SPELT);
        shaking || turning || popping || self.bounce().is_some() || card
    }

    /// The solving row hopping, if it is: which row, and how far up each of
    /// its tiles is.
    #[must_use]
    pub fn bounce(&self) -> Option<(usize, [bool; LEN])> {
        let round = self.round.as_ref()?;
        let (n, _) = round.me.solved_in()?;
        let start = round.me.guesses.get(n - 1)?.at + REVEAL;
        let now = self.now();
        if now < start || now >= start + BOUNCE {
            return None;
        }
        let t = now - start;
        let mut up = [false; LEN];
        for (i, lifted) in up.iter_mut().enumerate() {
            let from = HOP_STEP * u32::try_from(i).unwrap_or(0);
            *lifted = t >= from && t < from + HOP;
        }
        Some((n - 1, up))
    }

    /// When the card for a settled round comes up: once the guess that
    /// settled it has turned over, and a solving row has hopped.
    #[must_use]
    pub fn card_start(&self) -> Option<Instant> {
        let round = self.round.as_ref()?;
        if self.card_hidden {
            return None;
        }
        let settled = round.settled_at?;
        let hop = if round.me.solved_in().is_some() {
            BOUNCE
        } else {
            Duration::ZERO
        };
        Some(settled + REVEAL + hop + CARD_DELAY)
    }

    /// How long the card has been up, if it is.
    #[must_use]
    pub fn card_since(&self) -> Option<Duration> {
        let start = self.card_start()?;
        let now = self.now();
        (now >= start).then(|| now - start)
    }

    /// Whether the letter just typed at `i` still stands out.
    #[must_use]
    pub fn popping(&self, i: usize) -> bool {
        i + 1 == self.typing.len() && self.typed_at.is_some_and(|at| self.now() < at + POP)
    }

    /// How far the typing row is pushed sideways by its shake, in cells.
    #[must_use]
    pub fn shake_offset(&self) -> i16 {
        let Some(at) = self.shake else { return 0 };
        let t = self.now().saturating_duration_since(at).as_secs_f32() / SHAKE.as_secs_f32();
        if t >= 1.0 {
            return 0;
        }
        // Three quick swings either way, dying down.
        let swing = (t * 6.0 * std::f32::consts::PI).sin() * 2.0 * (1.0 - t);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a swing of at most two cells"
        )]
        let cells = swing.round() as i16;
        cells
    }

    /// A key the table passed on. Leaving, the mouse and the share code are
    /// the table's; while a guess is being typed, only esc gets back to it.
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx) -> Handled {
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if self.guessing() {
            match key.code {
                KeyCode::Char(c) if plain && c.is_ascii_alphabetic() => {
                    if self.typing.len() < LEN {
                        // Checked ASCII, so this is the byte itself.
                        self.typing.push(c.to_ascii_lowercase() as u8);
                        self.typed_at = Some(self.now());
                    }
                }
                KeyCode::Backspace => {
                    self.typing.pop();
                    self.typed_at = None;
                }
                KeyCode::Enter => self.submit(ctx),
                KeyCode::Tab if ctx.has_chat() => ctx.chat.focus(),
                _ => return Handled::Unused,
            }
            return Handled::Used;
        }
        // The card for a settled round: enter goes again, q still leaves, and
        // anything else puts it away to look at the boards.
        if self.card_since().is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => self.ready(ctx),
                KeyCode::Char('q') => return Handled::Unused,
                _ => self.card_hidden = true,
            }
            return Handled::Used;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') if !self.in_play() => self.ready(ctx),
            KeyCode::Char('h') if self.can_change_mode() => self.hard = !self.hard,
            KeyCode::Tab if ctx.has_chat() => ctx.chat.focus(),
            _ => return Handled::Unused,
        }
        Handled::Used
    }

    /// Whether `h` still changes the next round's mode: not during a round,
    /// and not once either side has offered one.
    #[must_use]
    pub fn can_change_mode(&self) -> bool {
        !self.in_play() && !self.gone && self.setup.mine.is_none() && self.setup.theirs.is_none()
    }

    pub fn on_mouse(&mut self, ev: MouseEvent, ctx: &mut Ctx) {
        if ev.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        // A click anywhere puts the card away.
        if self.card_since().is_some() {
            self.card_hidden = true;
            return;
        }
        let g = Geometry::of(ctx.area, ctx, self);
        let code = match g.key_at(ev.column, ev.row) {
            Some(Key::Letter(b)) => KeyCode::Char(char::from(b)),
            Some(Key::Enter) => KeyCode::Enter,
            Some(Key::Back) => KeyCode::Backspace,
            None => return,
        };
        ctx.note = None;
        self.on_key(KeyEvent::new(code, KeyModifiers::NONE), ctx);
    }

    /// The guess typed so far goes in, if it counts.
    fn submit(&mut self, ctx: &mut Ctx) {
        let now = self.now();
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let Ok(word) = Word::try_from(self.typing.as_slice()) else {
            ctx.note = Some("not enough letters".into());
            self.shake = Some(now);
            return;
        };
        match round.me.check(word, round.answer, round.hard) {
            Ok(marks) => {
                let ms = u64::try_from(now.saturating_duration_since(round.started).as_millis())
                    .unwrap_or(u64::MAX);
                round.me.guesses.push(Guess {
                    word,
                    marks,
                    ms,
                    at: now,
                });
                ctx.send(Msg::Guess { word, ms }.line());
                self.typing.clear();
                self.tally();
            }
            Err(why) => {
                ctx.note = Some(why);
                self.shake = Some(now);
            }
        }
    }

    /// Ready for the next round. Alone it starts at once; against someone it
    /// is an offer, in their mode if they have already made one.
    fn ready(&mut self, ctx: &mut Ctx) {
        self.card_hidden = true;
        if self.alone() {
            self.start(pick(&secret()), self.hard);
            return;
        }
        if self.gone || self.setup.mine.is_some() {
            return;
        }
        let hard = self.setup.theirs.map_or(self.hard, |(hard, _)| hard);
        self.offer(hard, ctx);
        self.settle(ctx);
    }

    /// Offers a round in this mode, promising a fresh secret. Before the
    /// opponent is here it waits, and goes when they arrive.
    fn offer(&mut self, hard: bool, ctx: &Ctx) {
        let secret = secret();
        self.hard = hard;
        self.setup.mine = Some((hard, secret));
        ctx.send(offer_line(hard, &secret));
    }

    /// The opponent is in: an offer made while waiting for them goes now.
    pub fn on_connect(&mut self, ctx: &mut Ctx) {
        if let Some((hard, secret)) = self.setup.mine {
            ctx.send(offer_line(hard, &secret));
        }
    }

    /// The opponent is gone, so the round they were in cannot be settled
    /// and there will be no next one. Anything already settled stands.
    pub fn on_disconnect(&mut self, _ctx: &mut Ctx) {
        if !self.alone() {
            self.gone = true;
            self.setup = Setup::default();
        }
    }

    /// Moves the next round along as far as both offers allow: reveals our
    /// secret once the offers agree, and starts the round once both secrets
    /// are out.
    fn settle(&mut self, ctx: &mut Ctx) {
        let (Some((mine, secret)), Some((theirs, _))) = (self.setup.mine, self.setup.theirs) else {
            return;
        };
        if mine != theirs {
            // Offers that cross in different modes would wait on each other
            // for ever, so the guest gives way to the host.
            if self.seat == Seat::Guest && !self.setup.revealed {
                self.offer(theirs, ctx);
                self.settle(ctx);
            }
            return;
        }
        if !self.setup.revealed {
            self.setup.revealed = true;
            ctx.send(Msg::Seed(secret).line());
        }
        let Some(their_secret) = self.setup.their_secret else {
            return;
        };
        let (host, guest) = if self.seat == Seat::Host {
            (secret, their_secret)
        } else {
            (their_secret, secret)
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"tuitui wordle round");
        hasher.update(&self.next_number().to_le_bytes());
        hasher.update(&host);
        hasher.update(&guest);
        self.setup = Setup::default();
        self.start(pick(hasher.finalize().as_bytes()), mine);
    }

    /// The number the next round will have.
    fn next_number(&self) -> u32 {
        self.round.as_ref().map_or(1, |r| r.number + 1)
    }

    fn start(&mut self, answer: Word, hard: bool) {
        self.round = Some(Round {
            number: self.next_number(),
            answer,
            hard,
            started: self.now(),
            me: Board::default(),
            them: (!self.alone()).then(Board::default),
            settled_at: None,
        });
        self.hard = hard;
        self.typing.clear();
        self.shake = None;
        self.card_hidden = false;
    }

    /// Puts a round that has just been settled into the score.
    fn tally(&mut self) {
        let now = self.now();
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let Some(outcome) = round.outcome().filter(|_| round.settled_at.is_none()) else {
            return;
        };
        round.settled_at = Some(now);
        let s = &mut self.score;
        match outcome {
            Outcome::Won => {
                s.won += 1;
                s.streak += 1;
                s.best_streak = s.best_streak.max(s.streak);
                if let Some((n, _)) = round.me.solved_in() {
                    s.spread[n - 1] += 1;
                }
            }
            Outcome::Lost => {
                s.lost += 1;
                s.streak = 0;
            }
            Outcome::Drawn => s.drawn += 1,
        }
    }

    /// A line from the opponent. Anything that is not one of Wordle's
    /// messages is ignored, so the protocol can grow.
    pub fn on_line(&mut self, line: &str, ctx: &mut Ctx) {
        if self.alone() {
            return;
        }
        if let Some(msg) = Msg::parse(line) {
            self.on_msg(msg, ctx);
        }
    }

    /// A message from the opponent, checked against where the match is
    /// before it changes anything.
    fn on_msg(&mut self, msg: Msg, ctx: &mut Ctx) {
        match msg {
            // Offers are for between rounds, and ours may not change under
            // us once our secret is out.
            Msg::Round { hard, commit } => {
                if self.in_play() || self.setup.revealed {
                    return;
                }
                self.setup.theirs = Some((hard, commit));
                self.setup.their_secret = None;
                self.settle(ctx);
            }
            Msg::Seed(secret) => {
                let Some((_, commit)) = self.setup.theirs else {
                    return;
                };
                if self.setup.their_secret.is_some() || self.in_play() {
                    return;
                }
                if promise(&secret) != commit {
                    ctx.note = Some("peer's secret does not match its promise".into());
                    return;
                }
                self.setup.their_secret = Some(secret);
                self.settle(ctx);
            }
            Msg::Guess { word, ms } => {
                let now = self.now();
                let Some(round) = self.round.as_mut().filter(|r| !r.over()) else {
                    return;
                };
                let Some(them) = round.them.as_mut() else {
                    return;
                };
                match them.check(word, round.answer, round.hard) {
                    Ok(marks) => them.guesses.push(Guess {
                        word,
                        marks,
                        ms,
                        at: now,
                    }),
                    // A peer running the same code cannot send this.
                    Err(why) => {
                        ctx.note = Some(format!("peer sent a guess that does not count: {why}"));
                        return;
                    }
                }
                self.tally();
            }
        }
    }

    /// What to say on the status line, and how loudly.
    pub fn status(&self, ctx: &Ctx) -> (String, Tone) {
        if let Some(note) = &ctx.note {
            return (note.clone(), Tone::Warn);
        }
        if let crate::games::Conn::Lost(why) = &ctx.conn {
            return (why.clone(), Tone::Warn);
        }
        let peer = ctx.peer_label();
        if let Some(r) = self.round.as_ref().filter(|r| !r.over()) {
            if !r.me.done() {
                let n = r.me.guesses.len() + 1;
                return (format!("guess {n} of {TRIES}"), Tone::Go);
            }
            let text = match r.me.solved_in() {
                Some((n, _)) => format!("solved in {n} — waiting on {peer}"),
                None => format!(
                    "the word was {} — waiting on {peer}",
                    words::shout(&r.answer)
                ),
            };
            return (text, Tone::Wait);
        }
        if !self.alone()
            && let Some(line) = self.readiness(ctx, &peer)
        {
            return line;
        }
        let Some(r) = &self.round else {
            let text = if self.alone() {
                "enter to start"
            } else {
                "enter when you're ready"
            };
            return (text.into(), Tone::Go);
        };
        let (verdict, tone) = Self::verdict(r, &peer);
        let next = if self.alone() {
            "enter for another word"
        } else {
            "enter for a rematch"
        };
        (format!("{verdict} · {next}"), tone)
    }

    /// Where agreeing the next round stands, if anyone has offered one.
    pub fn readiness(&self, ctx: &Ctx, peer: &str) -> Option<(String, Tone)> {
        let mode = |hard: bool| if hard { "hard" } else { "normal" };
        Some(match (self.setup.mine, self.setup.theirs) {
            (None, None) => return None,
            (Some(_), None) if !ctx.is_networked() => (
                "ready — the round starts when they arrive".into(),
                Tone::Wait,
            ),
            (Some(_), None) => (format!("waiting for {peer} to be ready"), Tone::Wait),
            (None, Some((hard, _))) => (
                format!("{peer} is ready for a {} round — enter to play", mode(hard)),
                Tone::Offer,
            ),
            (Some(_), Some(_)) => ("starting…".into(), Tone::Wait),
        })
    }

    /// How the round `r` went, in words.
    #[must_use]
    pub fn verdict(r: &Round, peer: &str) -> (String, Tone) {
        let word = words::shout(&r.answer);
        let guesses = |n: usize| {
            if n == 1 {
                "1 guess".into()
            } else {
                format!("{n} guesses")
            }
        };
        let Some(them) = &r.them else {
            return match r.me.solved_in() {
                Some((n, _)) => (format!("solved in {}", guesses(n)), Tone::Win),
                None => (format!("the word was {word}"), Tone::Lose),
            };
        };
        let mine = r.me.solved_in().map(|(n, _)| n);
        let theirs = them.solved_in().map(|(n, _)| n);
        match (r.outcome(), mine, theirs) {
            (Some(Outcome::Won), Some(a), Some(b)) if a == b => {
                (format!("you win — both in {a}, you sooner"), Tone::Win)
            }
            (Some(Outcome::Won), Some(a), Some(b)) => {
                (format!("you win — {a} to {peer}'s {b}"), Tone::Win)
            }
            (Some(Outcome::Won), Some(a), None) => {
                (format!("you win in {}", guesses(a)), Tone::Win)
            }
            (Some(Outcome::Lost), Some(a), Some(b)) if a == b => (
                format!("{peer} wins — both in {a}, {peer} sooner"),
                Tone::Lose,
            ),
            (Some(Outcome::Lost), Some(a), Some(b)) => {
                (format!("{peer} wins — {b} to your {a}"), Tone::Lose)
            }
            (Some(Outcome::Lost), _, Some(b)) => {
                (format!("{peer} wins in {}", guesses(b)), Tone::Lose)
            }
            (Some(Outcome::Drawn), Some(a), _) => (
                format!("a draw — both in {a}, to the millisecond"),
                Tone::Done,
            ),
            _ => (format!("nobody found {word} — a draw"), Tone::Done),
        }
    }
}

/// A fresh secret.
fn secret() -> Bytes {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS has no random source");
    bytes
}

/// The promise of a secret: its hash.
#[must_use]
pub fn promise(secret: &Bytes) -> Bytes {
    *blake3::hash(secret).as_bytes()
}

/// An offer of a round in this mode, promising `secret`.
fn offer_line(hard: bool, secret: &Bytes) -> String {
    Msg::Round {
        hard,
        commit: promise(secret),
    }
    .line()
}

/// The answer that `seed`, random bytes, picks. Taking a 64-bit number
/// modulo a couple of thousand favours no word by any amount that matters.
fn pick(seed: &Bytes) -> Word {
    let answers = words::answers();
    let mut first = [0u8; 8];
    first.copy_from_slice(&seed[..8]);
    let n = u64::from_le_bytes(first) % answers.len() as u64;
    answers[usize::try_from(n).unwrap_or(0)]
}
