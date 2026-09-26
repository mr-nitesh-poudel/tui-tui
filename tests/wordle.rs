//! Wordle: marking guesses, the word lists, what the two sides say, and a
//! race played out between two tables wired to each other.

use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tui_tui::games::wordle::app::{Outcome, promise};
use tui_tui::games::wordle::protocol::Msg;
use tui_tui::games::wordle::ui::{Geometry, Key, Side};
use tui_tui::games::wordle::words::{self, Mark, Word, hard_miss, score};
use tui_tui::games::wordle::{self as wordle, App};
use tui_tui::games::{Conn, Ctx, Kind, Leave, Seat, Table};
use tui_tui::net::{Net, NetEvent};

use Mark::{Absent as A, Correct as C, Present as P};

fn w(text: &str) -> Word {
    words::word(text).unwrap()
}

#[test]
fn a_guess_is_marked_as_wordle_marks_it() {
    assert_eq!(score(&w("crane"), &w("crane")), [C; 5]);
    assert_eq!(score(&w("trace"), &w("crate")), [P, C, C, P, C]);
    // One E in the answer: the guess's first E takes it, the second is grey.
    assert_eq!(score(&w("speed"), &w("abide")), [A, A, P, A, P]);
    // The E in its right place is marked first, so those before it are grey.
    assert_eq!(score(&w("geese"), &w("those")), [A, A, A, C, C]);
    // Two Ls guessed, two in the answer: both are found.
    assert_eq!(score(&w("llama"), &w("allay")), [P, C, P, A, P]);
}

#[test]
fn hard_mode_keeps_what_was_found() {
    let earlier = [(w("crate"), score(&w("crate"), &w("trace")))];
    assert_eq!(hard_miss(&earlier, &w("trace")), None);
    assert_eq!(
        hard_miss(&earlier, &w("brake")).as_deref(),
        Some("guess must contain C")
    );
    let greens = [(w("stamp"), score(&w("stamp"), &w("stomp")))];
    assert_eq!(
        hard_miss(&greens, &w("slump")).as_deref(),
        Some("2nd letter must be T")
    );
}

#[test]
fn the_word_lists_hold_together() {
    let answers = words::answers();
    assert!(answers.len() > 2000, "{} answers", answers.len());
    for a in answers {
        assert!(words::is_word(a), "{} is guessable", words::shout(a));
        assert!(a.iter().all(u8::is_ascii_lowercase));
    }
    assert!(words::is_word(&w("zesty")), "a guess not in the answers");
    assert!(!words::is_word(&w("xqzvw")));
    assert_eq!(words::word("CrAnE"), Some(w("crane")));
    assert_eq!(words::word("cran"), None);
    assert_eq!(words::word("crané"), None);
}

#[test]
fn messages_survive_the_trip_and_nonsense_does_not() {
    for msg in [
        Msg::Round {
            hard: true,
            commit: [7; 32],
        },
        Msg::Round {
            hard: false,
            commit: [0xab; 32],
        },
        Msg::Seed([1; 32]),
        Msg::Guess {
            word: w("crane"),
            ms: 12_345,
        },
    ] {
        assert_eq!(Msg::parse(&msg.line()), Some(msg), "{msg:?}");
    }
    for bad in [
        "",
        "round",
        "round tricky 00",
        &format!("round easy {}", "0".repeat(63)),
        &format!("round easy {}", "g".repeat(64)),
        &format!("seed {} extra", "0".repeat(64)),
        "guess crane",
        "guess CRANE 1",
        "guess cranes 1",
        "guess crane -1",
        "guess crane 1 2",
    ] {
        assert_eq!(Msg::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn wordle_is_in_the_registry_and_played_alone_locally() {
    assert_eq!(Kind::from_name("wordle"), Some(wordle::KIND));
    assert!(wordle::KIND.alone());
    assert!(!tui_tui::games::chess::KIND.alone());
}

fn press(t: &mut Table<App>, code: KeyCode) {
    t.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_in(t: &mut Table<App>, text: &str) {
    for c in text.chars() {
        press(t, KeyCode::Char(c));
    }
}

fn guess(t: &mut Table<App>, word: &str) {
    type_in(t, word);
    press(t, KeyCode::Enter);
}

fn answer(t: &Table<App>) -> String {
    let a = t.play.round.as_ref().expect("a round is on").answer;
    String::from_utf8(a.to_vec()).unwrap()
}

/// A real word that is not `t`'s answer, and shares no letter with it where
/// it can, so it solves nothing.
fn wrong(t: &Table<App>) -> String {
    let a = t.play.round.as_ref().unwrap().answer;
    let pick = words::answers()
        .iter()
        .find(|c| c.iter().all(|b| !a.contains(b)))
        .or_else(|| words::answers().iter().find(|&&c| c != a))
        .unwrap();
    String::from_utf8(pick.to_vec()).unwrap()
}

#[test]
fn alone_a_round_is_one_player_and_the_word() {
    let mut t = Table::local(App::new(Seat::Local));
    assert!(!t.play.in_play());
    press(&mut t, KeyCode::Enter);
    assert!(t.play.guessing());
    assert!(t.play.round.as_ref().unwrap().them.is_none());

    // q is a letter while guessing, and does not leave.
    type_in(&mut t, "q");
    assert_eq!(t.play.typing, b"q");
    assert_eq!(t.leaving(), None);
    press(&mut t, KeyCode::Backspace);

    let not_word = "xqzvw";
    guess(&mut t, not_word);
    assert!(
        t.ctx
            .note
            .as_deref()
            .unwrap()
            .contains("not in the word list")
    );
    assert!(t.play.round.as_ref().unwrap().me.guesses.is_empty());
    assert!(t.play.shake.is_some(), "the row shakes");
    press(&mut t, KeyCode::Backspace);
    for _ in 0..5 {
        press(&mut t, KeyCode::Backspace);
    }
    assert!(t.play.typing.is_empty());

    let miss = wrong(&t);
    guess(&mut t, &miss);
    let a = answer(&t);
    guess(&mut t, &a);
    let round = t.play.round.as_ref().unwrap();
    assert_eq!(round.outcome(), Some(Outcome::Won));
    assert_eq!(t.play.score.won, 1);
    assert_eq!(t.play.score.streak, 1);
    assert_eq!(t.play.score.spread[1], 1, "found in two");

    // Over, so leaving asks nothing, and enter brings the next word.
    press(&mut t, KeyCode::Enter);
    assert_eq!(t.play.round.as_ref().unwrap().number, 2);
    for _ in 0..6 {
        let miss = wrong(&t);
        guess(&mut t, &miss);
    }
    assert_eq!(
        t.play.round.as_ref().unwrap().outcome(),
        Some(Outcome::Lost)
    );
    assert_eq!(t.play.score.streak, 0);
    assert_eq!(t.play.score.best_streak, 1);
    press(&mut t, KeyCode::Char('q'));
    assert_eq!(t.leaving(), Some(Leave::Lobby));
}

#[test]
fn hard_mode_is_chosen_between_rounds_and_enforced() {
    let mut t = Table::local(App::new(Seat::Local));
    press(&mut t, KeyCode::Char('h'));
    assert!(t.play.hard);
    press(&mut t, KeyCode::Enter);
    let a = t.play.round.as_ref().unwrap().answer;
    // A first guess sharing a letter with the answer, then one dropping it.
    let first = words::answers()
        .iter()
        .find(|c| **c != a && c.iter().any(|b| a.contains(b)))
        .copied()
        .unwrap();
    guess(&mut t, std::str::from_utf8(&first).unwrap());
    let dropped = words::answers()
        .iter()
        .find(|c| hard_miss(&[(first, score(&first, &a))], c).is_some())
        .copied()
        .unwrap();
    guess(&mut t, std::str::from_utf8(&dropped).unwrap());
    assert_eq!(t.play.round.as_ref().unwrap().me.guesses.len(), 1);
    assert!(t.ctx.note.is_some(), "told why");
}

#[test]
fn clicking_the_keyboard_types() {
    let mut t = Table::local(App::new(Seat::Local));
    let area = Rect::new(0, 0, 100, 40);
    t.set_area(area);
    let g = Geometry::of(area, &t.ctx, &t.play);
    let at = |k: Key| {
        let r = g
            .key_rects()
            .into_iter()
            .find(|&(key, _)| key == k)
            .unwrap()
            .1;
        (r.x + 1, r.y)
    };
    let click = |t: &mut Table<App>, (column, row): (u16, u16)| {
        t.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    };
    click(&mut t, at(Key::Enter));
    assert!(t.play.guessing(), "enter on screen starts a round");
    for b in *b"crane" {
        click(&mut t, at(Key::Letter(b)));
    }
    assert_eq!(t.play.typing, b"crane");
    click(&mut t, at(Key::Back));
    assert_eq!(t.play.typing, b"cran");
}

/// Two tables wired to each other, and what each has sent and not yet been
/// delivered.
struct Pair {
    host: Table<App>,
    guest: Table<App>,
    to_guest: UnboundedReceiver<String>,
    to_host: UnboundedReceiver<String>,
}

fn table(seat: Seat) -> Table<App> {
    let mut t = Table::new(Ctx::new(Conn::Dialling, None), App::new(seat));
    t.set_area(Rect::new(0, 0, 120, 40));
    t
}

fn wire(t: &mut Table<App>, name: &str) -> UnboundedReceiver<String> {
    let (out, sent) = unbounded_channel();
    let peer = iroh::SecretKey::generate().public();
    t.attach(Net { peer, out }, name);
    sent
}

fn pair() -> Pair {
    let mut host = table(Seat::Host);
    let mut guest = table(Seat::Guest);
    let to_guest = wire(&mut host, "guest");
    let to_host = wire(&mut guest, "host");
    Pair {
        host,
        guest,
        to_guest,
        to_host,
    }
}

impl Pair {
    /// Delivers everything sent so far, both ways, until nothing is left.
    fn pump(&mut self) {
        loop {
            let mut moved = false;
            while let Ok(line) = self.to_guest.try_recv() {
                self.guest.on_net(NetEvent::Line(line));
                moved = true;
            }
            while let Ok(line) = self.to_host.try_recv() {
                self.host.on_net(NetEvent::Line(line));
                moved = true;
            }
            if !moved {
                return;
            }
        }
    }

    fn start(&mut self) {
        press(&mut self.host, KeyCode::Enter);
        press(&mut self.guest, KeyCode::Enter);
        self.pump();
    }
}

#[test]
fn both_sides_race_the_same_word() {
    let mut p = pair();
    press(&mut p.host, KeyCode::Enter);
    p.pump();
    assert!(p.host.play.round.is_none(), "one side ready is not a round");
    assert!(
        p.guest.play.setup.theirs.is_some(),
        "the guest hears the offer"
    );
    press(&mut p.guest, KeyCode::Enter);
    p.pump();
    let (h, g) = (answer(&p.host), answer(&p.guest));
    assert_eq!(h, g);
    assert!(p.host.play.guessing() && p.guest.play.guessing());

    // The next round draws a new word from new secrets, and still agrees.
    for _ in 0..6 {
        let wrong = wrong(&p.host);
        guess(&mut p.host, &wrong);
        guess(&mut p.guest, &wrong);
    }
    p.pump();
    assert!(!p.host.play.in_play() && !p.guest.play.in_play());
    p.start();
    assert_eq!(answer(&p.host), answer(&p.guest));
    assert_eq!(p.host.play.round.as_ref().unwrap().number, 2);
}

#[test]
fn an_offer_made_while_waiting_goes_when_they_arrive() {
    let mut host = Table::new(Ctx::new(Conn::Waiting, None), App::new(Seat::Host));
    press(&mut host, KeyCode::Char('h'));
    press(&mut host, KeyCode::Enter);
    let mut to_guest = wire(&mut host, "guest");
    let line = to_guest.try_recv().expect("the offer went on connecting");
    assert!(matches!(
        Msg::parse(&line),
        Some(Msg::Round { hard: true, .. })
    ));
}

#[test]
fn crossed_offers_settle_on_the_hosts_mode() {
    let mut p = pair();
    press(&mut p.host, KeyCode::Char('h'));
    p.start();
    let (h, g) = (
        p.host.play.round.as_ref().unwrap(),
        p.guest.play.round.as_ref().unwrap(),
    );
    assert!(h.hard && g.hard, "the guest gave way");
    assert_eq!(h.answer, g.answer);
}

#[test]
fn accepting_an_offer_takes_its_mode() {
    let mut p = pair();
    press(&mut p.guest, KeyCode::Char('h'));
    press(&mut p.guest, KeyCode::Enter);
    p.pump();
    // With an offer in, the mode is no longer the host's to change.
    assert!(!p.host.play.can_change_mode());
    press(&mut p.host, KeyCode::Enter);
    p.pump();
    assert!(p.host.play.round.as_ref().unwrap().hard);
    assert_eq!(answer(&p.host), answer(&p.guest));
}

#[test]
fn fewer_guesses_wins_and_both_sides_agree() {
    let mut p = pair();
    p.start();
    let a = answer(&p.host);
    let wrong = wrong(&p.host);
    guess(&mut p.host, &wrong);
    guess(&mut p.host, &a);
    p.pump();
    // The host found it in two; the guest is not done, and could still tie.
    assert_eq!(p.guest.play.round.as_ref().unwrap().outcome(), None);
    assert!(
        !p.guest.play.round.as_ref().unwrap().show_theirs(),
        "the host's letters stay hidden from someone still guessing"
    );
    guess(&mut p.guest, &wrong);
    guess(&mut p.guest, &wrong);
    p.pump();
    for (t, want) in [(&p.host, Outcome::Won), (&p.guest, Outcome::Lost)] {
        assert_eq!(t.play.round.as_ref().unwrap().outcome(), Some(want));
    }
    assert_eq!(p.host.play.score.won, 1);
    assert_eq!(p.guest.play.score.lost, 1);
    assert!(!p.guest.play.guessing(), "settled, so no more guesses");
}

#[test]
fn the_same_number_of_guesses_goes_to_whoever_was_sooner() {
    let mut p = pair();
    let t0 = Instant::now();
    p.host.play.clock = Some(t0);
    p.guest.play.clock = Some(t0);
    p.start();
    let a = answer(&p.host);
    p.host.play.clock = Some(t0 + Duration::from_secs(9));
    p.guest.play.clock = Some(t0 + Duration::from_secs(4));
    guess(&mut p.host, &a);
    guess(&mut p.guest, &a);
    p.pump();
    assert_eq!(
        p.guest.play.round.as_ref().unwrap().outcome(),
        Some(Outcome::Won)
    );
    assert_eq!(
        p.host.play.round.as_ref().unwrap().outcome(),
        Some(Outcome::Lost)
    );
}

#[test]
fn a_peer_is_not_taken_at_its_word() {
    let mut p = pair();
    let line = |t: &mut Table<App>, s: &str| t.on_net(NetEvent::Line(s.to_string()));

    // A guess with no round on, and a secret with no offer, change nothing.
    line(&mut p.host, "guess crane 10");
    line(&mut p.host, &Msg::Seed([3; 32]).line());
    assert!(p.host.play.round.is_none());
    assert!(p.host.play.setup.their_secret.is_none());

    // A secret that is not the one promised is refused.
    line(
        &mut p.host,
        &Msg::Round {
            hard: false,
            commit: promise(&[1; 32]),
        }
        .line(),
    );
    press(&mut p.host, KeyCode::Enter);
    line(&mut p.host, &Msg::Seed([2; 32]).line());
    assert!(p.host.play.round.is_none());
    assert!(p.host.ctx.note.as_deref().unwrap().contains("promise"));
    line(&mut p.host, &Msg::Seed([1; 32]).line());
    assert!(p.host.play.round.is_some(), "the right one is taken");

    // Guesses that are not words, or come after the peer is done, count for
    // nothing.
    let them = |t: &Table<App>| {
        t.play
            .round
            .as_ref()
            .unwrap()
            .them
            .as_ref()
            .unwrap()
            .guesses
            .len()
    };
    line(&mut p.host, "guess xqzvw 10");
    assert_eq!(them(&p.host), 0);
    let a = answer(&p.host);
    line(&mut p.host, &format!("guess {a} 10"));
    line(&mut p.host, &format!("guess {a} 20"));
    assert_eq!(them(&p.host), 1);
    // And an offer in the middle of a round is not one.
    line(&mut p.host, "garbage");
    assert!(p.host.play.round.as_ref().unwrap().them.is_some());
}

#[test]
fn screens_draw_at_any_size() {
    let mut p = pair();
    p.start();
    let wrong = wrong(&p.host);
    guess(&mut p.host, &wrong);
    type_in(&mut p.host, "ab");
    p.pump();
    let mut alone = Table::local(App::new(Seat::Local));
    press(&mut alone, KeyCode::Enter);
    guess(&mut alone, &wrong);
    let waiting = Table::new(Ctx::new(Conn::Waiting, None), App::new(Seat::Host));
    let mut tables = [p.host, p.guest, alone, waiting];
    for (w, h) in (0..=40)
        .step_by(3)
        .chain([80, 100, 160])
        .flat_map(|w| (0..=30).step_by(3).chain([40, 60]).map(move |h| (w, h)))
    {
        for t in &mut tables {
            let area = Rect::new(0, 0, w, h);
            t.set_area(area);
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| t.draw(f)).unwrap();
            for (x, y) in [
                (0, 0),
                (w / 2, h / 2),
                (w.saturating_sub(1), h.saturating_sub(1)),
            ] {
                t.on_mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: x,
                    row: y,
                    modifiers: KeyModifiers::NONE,
                });
            }
        }
    }
}

#[test]
fn the_keyboard_waits_for_the_tiles_to_turn() {
    let mut t = Table::local(App::new(Seat::Local));
    let t0 = Instant::now();
    t.play.clock = Some(t0);
    press(&mut t, KeyCode::Enter);
    let miss = wrong(&t);
    guess(&mut t, &miss);
    let board = &t.play.round.as_ref().unwrap().me;
    let first = miss.as_bytes()[0];
    assert_eq!(board.letter(first, t0), None, "nothing shown yet");
    let later = t0 + Duration::from_secs(2);
    assert!(board.letter(first, later).is_some(), "shown once turned");
}

#[test]
fn a_settled_round_brings_up_a_card_that_any_key_puts_away() {
    let mut t = Table::local(App::new(Seat::Local));
    let t0 = Instant::now();
    t.play.clock = Some(t0);
    press(&mut t, KeyCode::Enter);
    let a = answer(&t);
    guess(&mut t, &a);
    assert!(
        t.play.card_since().is_none(),
        "not while the row turns over"
    );
    assert!(t.play.is_animating());
    t.play.clock = Some(t0 + Duration::from_secs(5));
    assert!(t.play.card_since().is_some());

    // Any key but enter or q just puts it away, to see the boards.
    press(&mut t, KeyCode::Char('x'));
    assert!(t.play.card_since().is_none());
    assert_eq!(t.play.round.as_ref().unwrap().number, 1);
    assert_eq!(t.leaving(), None);

    // Enter on it goes straight to the next word.
    press(&mut t, KeyCode::Enter);
    let a = answer(&t);
    t.play.clock = Some(t0 + Duration::from_secs(6));
    guess(&mut t, &a);
    t.play.clock = Some(t0 + Duration::from_secs(11));
    assert!(t.play.card_since().is_some());
    press(&mut t, KeyCode::Enter);
    assert_eq!(t.play.round.as_ref().unwrap().number, 3);
}

#[test]
fn the_tiles_grow_with_the_screen() {
    let small = Geometry::layout(Rect::new(0, 0, 80, 24), false, Side::Stats);
    let big = Geometry::layout(Rect::new(0, 0, 200, 60), false, Side::Stats);
    assert!(big.tile.0 > small.tile.0 && big.tile.1 > small.tile.1);
    assert!(big.key.1 > small.key.1, "the keys too");
    // Against someone, their board is small until the round is settled.
    let during = Geometry::layout(Rect::new(0, 0, 160, 50), true, Side::Theirs { full: false });
    let after = Geometry::layout(Rect::new(0, 0, 160, 50), true, Side::Theirs { full: true });
    assert!(during.side_tile.0 < during.tile.0);
    assert!(after.side_tile.1 > during.side_tile.1, "theirs grows");
    let wide = Geometry::layout(Rect::new(0, 0, 250, 60), true, Side::Theirs { full: true });
    assert_eq!(wide.side_tile, wide.tile, "to match ours, given room");
}

#[test]
fn an_opponent_who_leaves_ends_the_round_without_a_result() {
    let mut p = pair();
    p.start();
    let miss = wrong(&p.host);
    guess(&mut p.host, &miss);
    p.host.lost("your opponent left".into());
    assert!(!p.host.play.in_play(), "nothing left to lose by leaving");
    assert!(!p.host.play.guessing(), "and nobody to race");
    assert_eq!(
        p.host.play.score.played(),
        0,
        "an unfinished round counts for nothing"
    );
    assert!(!p.host.play.can_change_mode());
    press(&mut p.host, KeyCode::Enter);
    assert!(p.host.play.setup.mine.is_none(), "no rematch to offer");
    press(&mut p.host, KeyCode::Char('q'));
    assert_eq!(
        p.host.leaving(),
        Some(Leave::Lobby),
        "and q leaves without asking"
    );
}
