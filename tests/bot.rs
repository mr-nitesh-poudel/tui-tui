//! Playing the bot: picking its strength and your side, the engine held to
//! that strength, its moves, and what happens when it misbehaves.
//!
//! The engines here are a few lines of shell that speak just enough UCI, so
//! the tests need no Stockfish.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use shakmaty::{Color, Square};
use tui_tui::games::chess::App;
use tui_tui::games::chess::bot::{Bot, ELO_MAX, ELO_MIN, Pick, Setting};
use tui_tui::games::chess::engine::Status;
use tui_tui::games::chess::ui::Geometry;
use tui_tui::games::{Leave, Table};

/// Answers e2e4 for white and e7e5 for black, and says it can be held to
/// between 1320 and 2000. Everything it is told goes into `log`.
const FAKE: &str = r#"#!/bin/sh
fen=""
while read -r line; do
  echo "$line" >> "$(dirname "$0")/log"
  case "$line" in
    uci)
      echo "id name fake"
      echo "option name UCI_LimitStrength type check default false"
      echo "option name UCI_Elo type spin default 1320 min 1320 max 2000"
      echo "uciok" ;;
    "position fen "*) fen="${line#position fen }" ;;
    go*)
      case "$fen" in
        *" b "*) echo "info depth 5 score cp 20 pv e7e5"; echo "bestmove e7e5" ;;
        *) echo "bestmove e2e4" ;;
      esac ;;
    quit) exit 0 ;;
  esac
done
"#;

/// An engine from `body`, written somewhere of its own.
fn engine(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tuitui-{}-bot-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("engine");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// What the engine at `path` has been told.
fn told(path: &std::path::Path) -> String {
    std::fs::read_to_string(path.with_file_name("log")).unwrap_or_default()
}

/// Waits for `ready`, failing loudly rather than hanging.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..500 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("gave up waiting for {what}");
}

/// Waits for the bot to move, as the main loop would: whenever woken.
async fn until_played(t: &mut Table<App>, plies: usize) {
    for _ in 0..500 {
        t.on_wake();
        if t.play.game.plies() >= plies {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("gave up waiting for the bot to move");
}

fn press(t: &mut Table<App>, code: KeyCode) {
    t.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// Moves a piece the way a player at the keyboard would.
fn play(t: &mut Table<App>, from: Square, to: Square) {
    t.play.game.cursor = from;
    press(t, KeyCode::Enter);
    t.play.game.cursor = to;
    press(t, KeyCode::Enter);
}

fn state(t: &Table<App>) -> String {
    t.play.state_line(&t.ctx).0
}

fn setup(t: &Table<App>) -> (u16, Pick) {
    let v = t.play.versus.as_ref().unwrap();
    (v.elo, v.pick)
}

#[test]
fn the_strength_and_side_are_picked_first() {
    let mut t = Table::local(App::against_bot());
    assert_eq!(t.play.choosing(), Some(Setting::Strength));
    assert_eq!(setup(&t), (1500, Pick::White));
    assert!(!t.play.in_play(), "nothing to walk out on yet");
    assert_eq!(state(&t), "pick a strength and a side");

    press(&mut t, KeyCode::Right);
    assert_eq!(setup(&t).0, 1550);
    press(&mut t, KeyCode::Left);
    press(&mut t, KeyCode::Left);
    assert_eq!(setup(&t).0, 1450);
    for _ in 0..100 {
        press(&mut t, KeyCode::Left);
    }
    assert_eq!(setup(&t).0, ELO_MIN);
    for _ in 0..100 {
        press(&mut t, KeyCode::Right);
    }
    assert_eq!(setup(&t).0, ELO_MAX);

    press(&mut t, KeyCode::Down);
    assert_eq!(t.play.choosing(), Some(Setting::Side));
    press(&mut t, KeyCode::Right);
    assert_eq!(setup(&t).1, Pick::Black);
    // The board turns round to show it.
    assert_eq!(t.play.me, Some(Color::Black));
    assert!(t.play.flipped);
    press(&mut t, KeyCode::Right);
    assert_eq!(setup(&t).1, Pick::Random);
    press(&mut t, KeyCode::Up);
    assert_eq!(t.play.choosing(), Some(Setting::Strength));

    // The board is not in play: no pieces move, and nothing is resigned.
    t.play.game.cursor = Square::E2;
    press(&mut t, KeyCode::Char('r'));
    assert!(!t.play.confirm_resign);
    assert_eq!(t.play.game.selected, None);

    // q leaves at once, with no game to lose.
    press(&mut t, KeyCode::Char('q'));
    assert!(!t.ctx.confirm_leave);
    assert_eq!(t.leaving(), Some(Leave::Lobby));
}

#[test]
fn the_choices_can_be_clicked() {
    let mut t = Table::local(App::against_bot());
    let area = Rect::new(0, 0, 120, 40);
    t.set_area(area);
    let g = Geometry::for_game(area, &t.ctx, &t.play).setup;
    let click = |t: &mut Table<App>, r: Rect| {
        assert!(!r.is_empty(), "there is room for it");
        t.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.x + r.width / 2,
            row: r.y,
            modifiers: KeyModifiers::NONE,
        });
    };
    click(&mut t, g.more[0]);
    assert_eq!(setup(&t).0, 1550);
    click(&mut t, g.less[0]);
    click(&mut t, g.less[0]);
    assert_eq!(setup(&t).0, 1450);
    click(&mut t, g.labels[1]);
    assert_eq!(t.play.choosing(), Some(Setting::Side));
    click(&mut t, g.less[1]);
    assert_eq!(setup(&t).1, Pick::Random);
    assert_eq!(t.play.game.selected, None, "nothing under it was touched");

    // Drawn where it is clicked: the value between its arrows.
    let mut term = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    term.draw(|f| t.draw(f)).unwrap();
    let buf = term.backend().buffer();
    let text = |r: Rect| -> String {
        (r.x..r.right())
            .map(|x| buf[(x, r.y)].symbol().to_string())
            .collect()
    };
    assert_eq!(text(g.values[0]).trim(), "1450 Elo");
    assert_eq!(text(g.values[1]).trim(), "random");
    assert_eq!(text(g.less[0]).trim(), "◂");
    assert_eq!(text(g.more[1]).trim(), "▸");
    assert_eq!(text(g.start).trim(), "start");
}

#[tokio::test]
async fn the_bot_is_held_to_what_the_engine_can_do_and_answers_your_moves() {
    let path = engine("plays", FAKE);
    let mut t = Table::local(App::against_bot());
    t.play.versus.as_mut().unwrap().elo = 2500;
    t.play.start_bot(&path, &mut t.ctx);
    assert_eq!(t.play.choosing(), None);
    assert!(t.play.in_play());
    assert_eq!(t.play.me, Some(Color::White));
    assert!(!t.play.flipped);
    assert_eq!(state(&t), "your move");

    let bot = t.play.bot().unwrap();
    until("the bot to be ready", || bot.status() == Status::Ready).await;
    // This engine goes no higher than 2000, so that is what it plays at.
    assert_eq!(bot.elo(), Some(2000));
    assert_eq!(t.play.versus.as_ref().unwrap().title(), "bot · 2000");
    until("the strength to be set", || {
        told(&path).contains("setoption name UCI_Elo value 2000")
    })
    .await;
    let log = told(&path);
    assert!(
        log.contains("setoption name UCI_LimitStrength value true"),
        "{log}"
    );
    assert!(
        !log.contains("go"),
        "nothing asked while it is your move: {log}"
    );

    play(&mut t, Square::E2, Square::E4);
    assert_eq!(t.play.game.plies(), 1);
    assert_eq!(state(&t), "the bot is thinking…");
    until_played(&mut t, 2).await;
    assert_eq!(t.play.game.last, Some((Square::E7, Square::E5)));
    assert_eq!(state(&t), "your move");

    // Advice is open throughout, since it can only help the one playing.
    assert!(t.play.can_analyse());
}

#[tokio::test]
async fn playing_black_the_bot_goes_first() {
    let path = engine("first", FAKE);
    let mut t = Table::local(App::against_bot());
    t.play.versus.as_mut().unwrap().pick = Pick::Black;
    t.play.start_bot(&path, &mut t.ctx);
    assert_eq!(t.play.me, Some(Color::Black));
    assert!(t.play.flipped, "you sit behind your own pieces");
    until_played(&mut t, 1).await;
    assert_eq!(t.play.game.last, Some((Square::E2, Square::E4)));

    // Its pieces are not yours to move, and neither is its turn.
    play(&mut t, Square::D2, Square::D4);
    assert_eq!(t.play.game.plies(), 1);
}

#[tokio::test]
async fn a_move_for_a_position_the_game_has_left_is_thrown_away() {
    let path = engine("stale", FAKE);
    let woken = Arc::new(AtomicUsize::new(0));
    let wake = {
        let woken = woken.clone();
        Arc::new(move || {
            woken.fetch_add(1, Ordering::SeqCst);
        })
    };
    let bot = Bot::start(&path, 1500, Some(wake)).unwrap();
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let other = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
    bot.ask(start.into());
    // Once for being ready, once for the move.
    until("the move", || woken.load(Ordering::SeqCst) >= 2).await;
    assert_eq!(bot.reply(other), None, "not for this position");
    assert_eq!(bot.reply(start), None, "and not kept for later");

    bot.ask(start.into());
    until("the move again", || woken.load(Ordering::SeqCst) >= 3).await;
    assert_eq!(bot.reply(start).as_deref(), Some("e2e4"));
}

#[tokio::test]
async fn an_engine_that_cannot_be_held_back_plays_at_full_strength() {
    let path = engine(
        "full",
        "#!/bin/sh\nwhile read -r l; do case \"$l\" in uci) echo uciok;; go*) echo bestmove e2e4;; quit) exit;; esac; done\n",
    );
    let mut t = Table::local(App::against_bot());
    t.play.start_bot(&path, &mut t.ctx);
    let bot = t.play.bot().unwrap();
    until("the bot to be ready", || bot.status() == Status::Ready).await;
    assert_eq!(bot.elo(), None);
    assert_eq!(
        t.play.versus.as_ref().unwrap().title(),
        "bot · full strength"
    );
}

#[tokio::test]
async fn an_illegal_move_from_the_bot_is_refused() {
    let path = engine(
        "cheat",
        "#!/bin/sh\nwhile read -r l; do case \"$l\" in uci) echo uciok;; go*) echo bestmove e7e4;; quit) exit;; esac; done\n",
    );
    let mut t = Table::local(App::against_bot());
    t.play.start_bot(&path, &mut t.ctx);
    play(&mut t, Square::E2, Square::E4);
    for _ in 0..500 {
        t.on_wake();
        if t.ctx.note.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        t.ctx.note.as_deref(),
        Some("the bot sent illegal move \"e7e4\"")
    );
    assert_eq!(t.play.game.plies(), 1, "the board is as it was");
}

#[tokio::test]
async fn a_bot_that_dies_says_so() {
    let path = engine(
        "dies",
        "#!/bin/sh\nwhile read -r l; do case \"$l\" in uci) echo uciok;; go*) exit 3;; esac; done\n",
    );
    let mut t = Table::local(App::against_bot());
    t.play.versus.as_mut().unwrap().pick = Pick::Black;
    t.play.start_bot(&path, &mut t.ctx);
    let bot = t.play.bot().unwrap();
    until("the bot to be missed", || {
        matches!(bot.status(), Status::Failed(_))
    })
    .await;
    assert!(state(&t).contains("engine"), "{}", state(&t));
}

#[test]
fn a_bot_needs_somewhere_to_run() {
    // Outside the async runtime, the choice stays open and says why.
    let mut t = Table::local(App::against_bot());
    t.play.start_bot(&engine("no-runtime", FAKE), &mut t.ctx);
    assert_eq!(t.play.choosing(), Some(Setting::Strength));
    assert_eq!(
        t.ctx.note.as_deref(),
        Some("the bot needs the async runtime")
    );
}
