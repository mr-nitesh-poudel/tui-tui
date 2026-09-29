//! Playing the bot: the strength and side picked before the game starts, and
//! an engine held to that strength playing the other side.
//!
//! The bot is an engine process of its own, apart from the one that
//! analyses: an engine held back to a club player's strength is no use for
//! advice. Analysis is open throughout a game against the bot, since the only
//! person it can help is the one playing.
//!
//! The bot is asked for a move whenever it is its turn. What it answers is
//! checked like anything a peer sends: a move for a position the game has
//! left is dropped, and one the rules do not allow is refused.

use std::path::Path;
use std::sync::{Arc, Mutex};

use ratatui::crossterm::event::KeyCode;
use shakmaty::Color;
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::app::App;
use super::download;
use super::engine::{self, Said, Status, hello, launch, lock, send};
use super::ui::{Geometry, SetupHit};
use crate::games::{Ctx, Handled, Waker};
use crate::session::lines::Lines;

/// The weakest and strongest the bot can be asked to play, in Elo, and the
/// steps between. Stockfish plays no weaker than 1320.
pub const ELO_MIN: u16 = 1350;
pub const ELO_MAX: u16 = 3150;
pub const ELO_STEP: u16 = 50;
/// Where the strength starts: a casual player's.
pub const ELO_DEFAULT: u16 = 1500;
/// How long the bot thinks about each move, in milliseconds.
const THINK: u32 = 1000;

/// Which side the player asked to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    White,
    Black,
    Random,
}

impl Pick {
    const ALL: [Pick; 3] = [Pick::White, Pick::Black, Pick::Random];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Pick::White => "white",
            Pick::Black => "black",
            Pick::Random => "random",
        }
    }

    /// The pick `step` along, wrapping round at either end.
    fn next(self, step: isize) -> Self {
        let n = Self::ALL.len().cast_signed();
        let at = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(at.cast_signed() + step).rem_euclid(n).cast_unsigned()]
    }

    /// The side played: a coin toss for [`Pick::Random`], and white if there
    /// is no coin to toss.
    fn side(self) -> Color {
        match self {
            Pick::White => Color::White,
            Pick::Black => Color::Black,
            Pick::Random => {
                let mut coin = [0u8];
                match getrandom::fill(&mut coin) {
                    Ok(()) if coin[0] & 1 == 1 => Color::Black,
                    _ => Color::White,
                }
            }
        }
    }
}

/// One of the things picked before the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Strength,
    Side,
}

impl Setting {
    /// In the order they are listed.
    pub const ALL: [Setting; 2] = [Setting::Strength, Setting::Side];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Setting::Strength => "strength",
            Setting::Side => "play as",
        }
    }

    fn other(self) -> Self {
        match self {
            Setting::Strength => Setting::Side,
            Setting::Side => Setting::Strength,
        }
    }
}

/// A game against the bot.
pub struct Versus {
    /// How strong the bot was asked to play, in Elo.
    pub elo: u16,
    pub pick: Pick,
    /// Before the game starts, the setting being changed; `None` once it has.
    pub choosing: Option<Setting>,
    /// The engine playing the other side, once the game has started.
    pub bot: Option<Bot>,
}

impl Default for Versus {
    fn default() -> Self {
        Self {
            elo: ELO_DEFAULT,
            pick: Pick::White,
            choosing: Some(Setting::Strength),
            bot: None,
        }
    }
}

impl Versus {
    /// Moves a setting `step` along: the strength by [`ELO_STEP`], held in
    /// range, and the side round its choices.
    pub fn change(&mut self, setting: Setting, step: isize) {
        match setting {
            Setting::Strength => {
                let elo = if step < 0 {
                    self.elo.saturating_sub(ELO_STEP)
                } else {
                    self.elo.saturating_add(ELO_STEP)
                };
                self.elo = elo.clamp(ELO_MIN, ELO_MAX);
            }
            Setting::Side => self.pick = self.pick.next(step),
        }
    }

    /// What a setting is set to, as it is shown.
    #[must_use]
    pub fn value(&self, setting: Setting) -> String {
        match setting {
            Setting::Strength => format!("{} Elo", self.elo),
            Setting::Side => self.pick.label().into(),
        }
    }

    /// What the bot's card is headed with: how strong it is playing, once the
    /// engine has said, else how strong it was asked to.
    #[must_use]
    pub fn title(&self) -> String {
        match self.bot.as_ref().map_or(Some(self.elo), Bot::elo) {
            Some(elo) => format!("bot · {elo}"),
            None => "bot · full strength".into(),
        }
    }
}

/// An engine playing one side, held to a strength. Dropping it stops the
/// task, which ends the process.
pub struct Bot {
    asks: UnboundedSender<String>,
    status: Arc<Mutex<Status>>,
    /// The strength it plays at: as asked until the engine has said what it
    /// can do, then what it agreed to, or `None` if it cannot be held back.
    elo: Arc<Mutex<Option<u16>>>,
    /// The move it chose, and the position it chose it in.
    reply: Arc<Mutex<Option<(String, String)>>>,
}

impl Bot {
    /// Starts the engine at `path` to play at `elo`, or as near as it can,
    /// calling `wake` whenever it has something to say.
    ///
    /// # Errors
    ///
    /// If there is no async runtime, or the program cannot be started.
    pub fn start(path: &Path, elo: u16, wake: Option<Waker>) -> Result<Self, String> {
        let (asks, asked) = unbounded_channel();
        let bot = Self {
            asks,
            status: Arc::new(Mutex::new(Status::Starting)),
            elo: Arc::new(Mutex::new(Some(elo))),
            reply: Arc::default(),
        };
        let shared = Shared {
            status: bot.status.clone(),
            elo: bot.elo.clone(),
            reply: bot.reply.clone(),
            wake: wake.unwrap_or_else(|| Arc::new(|| {})),
        };
        launch(
            path,
            "the bot",
            bot.status.clone(),
            shared.wake.clone(),
            move |stdin, lines| async move { drive(stdin, lines, asked, elo, &shared).await },
        )?;
        Ok(bot)
    }

    /// Asks for a move in this position. Whatever it was thinking about
    /// before is given up.
    pub fn ask(&self, fen: String) {
        let _ = self.asks.send(fen);
    }

    /// The move the bot chose in `fen`, in UCI, if it has chosen one since
    /// last asked. A move chosen for any other position is thrown away.
    #[must_use]
    pub fn reply(&self, fen: &str) -> Option<String> {
        let (asked, best) = lock(&self.reply).take()?;
        (asked == fen).then_some(best)
    }

    #[must_use]
    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    /// How strong it is playing, in Elo, or `None` at full strength.
    #[must_use]
    pub fn elo(&self) -> Option<u16> {
        *lock(&self.elo)
    }
}

/// What the bot's task shares with the game.
struct Shared {
    status: Arc<Mutex<Status>>,
    elo: Arc<Mutex<Option<u16>>>,
    reply: Arc<Mutex<Option<(String, String)>>>,
    wake: Waker,
}

/// Talks to the engine until the game lets go of it: holds it to `elo`,
/// then answers each position asked about with a move.
async fn drive(
    mut stdin: ChildStdin,
    mut lines: Lines<ChildStdout>,
    mut asked: UnboundedReceiver<String>,
    elo: u16,
    shared: &Shared,
) -> Result<(), String> {
    let gone = |e: std::io::Error| format!("the engine stopped: {e}");
    let (mut can_limit, mut range) = (false, None);
    hello(&mut stdin, &mut lines, |line| match option(line) {
        Some(("UCI_LimitStrength", _)) => can_limit = true,
        Some(("UCI_Elo", rest)) => range = elo_range(rest),
        _ => {}
    })
    .await?;
    // An engine that cannot be held back plays at its best.
    let playing = match range.filter(|_| can_limit) {
        Some((low, high)) => {
            let elo = elo.clamp(low, high);
            send(&mut stdin, "setoption name UCI_LimitStrength value true").await?;
            send(&mut stdin, &format!("setoption name UCI_Elo value {elo}")).await?;
            Some(elo)
        }
        None => None,
    };
    *lock(&shared.elo) = playing;
    *lock(&shared.status) = Status::Ready;
    (shared.wake)();

    // The position being searched, and the one to search next.
    let mut running: Option<String> = None;
    let mut wanted: Option<String> = None;
    loop {
        if running.is_none()
            && let Some(fen) = wanted.take()
        {
            send(&mut stdin, &format!("position fen {fen}")).await?;
            send(&mut stdin, &format!("go movetime {THINK}")).await?;
            running = Some(fen);
        }

        tokio::select! {
            ask = asked.recv() => match ask {
                // The game has let go: ask the engine to leave, and the
                // process is killed on the way out either way.
                None => {
                    let _ = send(&mut stdin, "quit").await;
                    return Ok(());
                }
                Some(fen) if running.as_ref() == Some(&fen) => wanted = None,
                Some(fen) => {
                    // Stop the search under way, once, and search this
                    // when it has.
                    if running.is_some() && wanted.is_none() {
                        send(&mut stdin, "stop").await?;
                    }
                    wanted = Some(fen);
                }
            },
            line = lines.next_line() => {
                let line = line.map_err(gone)?.ok_or("the engine quit")?;
                if let Some(Said::BestMove(best)) = engine::parse(&line)
                    && let Some(fen) = running.take()
                    // A search given up for another position is of no use.
                    && wanted.is_none()
                    && let Some(best) = best
                {
                    *lock(&shared.reply) = Some((fen, best));
                    (shared.wake)();
                }
            }
        }
    }
}

/// The name of an option the engine lists, and what it says after the name:
/// `option name Skill Level type spin …` is `("Skill Level", "type spin …")`.
fn option(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("option")?.trim_start();
    let rest = rest.strip_prefix("name")?.trim_start();
    let at = rest.find(" type ")?;
    Some((rest[..at].trim(), rest[at..].trim()))
}

/// The lowest and highest Elo an engine can be held to, from what it says
/// about `UCI_Elo`: `type spin default 1320 min 1320 max 3190`.
fn elo_range(said: &str) -> Option<(u16, u16)> {
    let mut words = said.split_whitespace();
    let (mut min, mut max) = (None, None);
    while let Some(word) = words.next() {
        match word {
            "min" => min = words.next().and_then(|n| n.parse::<u16>().ok()),
            "max" => max = words.next().and_then(|n| n.parse::<u16>().ok()),
            _ => {}
        }
    }
    let (min, max) = (min?, max?);
    (min <= max).then_some((min, max))
}

impl App {
    /// A game against the bot, starting with its strength and side to pick.
    #[must_use]
    pub fn against_bot() -> Self {
        let mut app = Self::local();
        app.me = Some(Color::White);
        app.versus = Some(Versus::default());
        app
    }

    /// Whether the strength and side are still being picked, before a game
    /// against the bot starts.
    #[must_use]
    pub fn choosing(&self) -> Option<Setting> {
        self.versus.as_ref()?.choosing
    }

    /// The engine playing the other side, once the game has started.
    #[must_use]
    pub fn bot(&self) -> Option<&Bot> {
        self.versus.as_ref()?.bot.as_ref()
    }

    /// A key while picking the strength and side. Leaving and the mouse are
    /// still the table's.
    pub(super) fn setup_key(&mut self, code: KeyCode, ctx: &mut Ctx) -> Handled {
        let Some(v) = self.versus.as_mut() else {
            return Handled::Unused;
        };
        let Some(setting) = v.choosing else {
            return Handled::Unused;
        };
        match code {
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Char('k' | 'j') => v.choosing = Some(setting.other()),
            KeyCode::Left | KeyCode::Char('h') => v.change(setting, -1),
            KeyCode::Right | KeyCode::Char('l') => v.change(setting, 1),
            KeyCode::Enter | KeyCode::Char(' ') => self.begin(ctx),
            _ => return Handled::Unused,
        }
        self.sit();
        Handled::Used
    }

    /// Sits the player behind the side they picked, so the board shows it
    /// while they choose. A random side is settled when the game starts.
    fn sit(&mut self) {
        let side = match self.versus.as_ref().map(|v| (v.choosing, v.pick)) {
            Some((Some(_), Pick::White)) => Color::White,
            Some((Some(_), Pick::Black)) => Color::Black,
            _ => return,
        };
        self.me = Some(side);
        self.flipped = side == Color::Black;
    }

    /// Starts the game against the bot, if there is an engine to play it,
    /// else offers to download one.
    pub(super) fn begin(&mut self, ctx: &mut Ctx) {
        // One is on its way, and starts the game when it arrives.
        if self.download.is_some() {
            return;
        }
        let Some(path) = engine::locate() else {
            if download::this_build().is_some() {
                self.offer_download = true;
            } else {
                ctx.note = Some(engine::INSTALL_HINT.into());
            }
            return;
        };
        self.start_bot(&path, ctx);
    }

    /// Starts the engine at `path` playing as the bot, and with it the game.
    /// If it cannot be started, the choice stays open and says why.
    pub fn start_bot(&mut self, path: &Path, ctx: &mut Ctx) {
        let Some(v) = self.versus.as_mut() else {
            return;
        };
        match Bot::start(path, v.elo, ctx.waker.clone()) {
            Ok(bot) => {
                v.bot = Some(bot);
                v.choosing = None;
                let me = v.pick.side();
                self.me = Some(me);
                // Sit behind your own pieces, as against anyone.
                self.flipped = me == Color::Black;
                self.prompt_bot();
            }
            Err(why) => ctx.note = Some(why),
        }
    }

    /// A click while picking the strength and side: an arrow changes a
    /// setting, a line picks it out, and the button starts the game.
    pub(super) fn setup_click(&mut self, g: &Geometry, x: u16, y: u16, ctx: &mut Ctx) {
        let Some(hit) = g.setup.at(x, y) else { return };
        let Some(v) = self.versus.as_mut() else {
            return;
        };
        match hit {
            SetupHit::Less(setting) => {
                v.choosing = Some(setting);
                v.change(setting, -1);
            }
            SetupHit::More(setting) => {
                v.choosing = Some(setting);
                v.change(setting, 1);
            }
            SetupHit::Line(setting) => v.choosing = Some(setting),
            SetupHit::Start => self.begin(ctx),
        }
        self.sit();
    }

    /// Asks the bot for a move, if it is its turn.
    pub(super) fn prompt_bot(&self) {
        if let Some(bot) = self.bot()
            && self.in_play()
            && !self.my_turn()
        {
            bot.ask(self.game.fen_at(self.game.plies()));
        }
    }

    /// Plays the bot's move, if it has chosen one for the position now and
    /// it is still its turn.
    pub(super) fn bot_moves(&mut self, ctx: &mut Ctx) {
        let Some(bot) = self.bot() else { return };
        let Some(uci) = bot.reply(&self.game.fen_at(self.game.plies())) else {
            return;
        };
        if !self.in_play() || self.my_turn() {
            return;
        }
        match self.game.play_uci(&uci) {
            Ok(m) => self.begin_slide(m),
            Err(why) => ctx.note = Some(format!("the bot sent {why}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_read_by_name() {
        assert_eq!(
            option("option name UCI_Elo type spin default 1320 min 1320 max 3190"),
            Some(("UCI_Elo", "type spin default 1320 min 1320 max 3190"))
        );
        assert_eq!(
            option("option name Skill Level type spin default 20 min 0 max 20"),
            Some(("Skill Level", "type spin default 20 min 0 max 20"))
        );
        assert_eq!(option("id name Stockfish"), None);
        assert_eq!(option("option name Broken"), None);
    }

    #[test]
    fn the_elo_range_is_what_the_engine_says_or_nothing() {
        let range = |line: &str| option(line).and_then(|(_, rest)| elo_range(rest));
        assert_eq!(
            range("option name UCI_Elo type spin default 1320 min 1320 max 3190"),
            Some((1320, 3190))
        );
        for wild in [
            "option name UCI_Elo type spin default 1 min 3000 max 1000",
            "option name UCI_Elo type spin default 1 min -5 max 1000",
            "option name UCI_Elo type spin default 1 min 1 max 99999999",
            "option name UCI_Elo type spin default 1 min",
            "option name UCI_Elo type check default false",
        ] {
            assert_eq!(range(wild), None, "{wild:?}");
        }
    }

    #[test]
    fn the_strength_stays_in_range_and_the_side_goes_round() {
        let mut v = Versus::default();
        for _ in 0..100 {
            v.change(Setting::Strength, -1);
        }
        assert_eq!(v.elo, ELO_MIN);
        for _ in 0..100 {
            v.change(Setting::Strength, 1);
        }
        assert_eq!(v.elo, ELO_MAX);
        assert_eq!(v.pick, Pick::White);
        v.change(Setting::Side, -1);
        assert_eq!(v.pick, Pick::Random);
        v.change(Setting::Side, 1);
        v.change(Setting::Side, 1);
        assert_eq!(v.pick, Pick::Black);
    }
}
