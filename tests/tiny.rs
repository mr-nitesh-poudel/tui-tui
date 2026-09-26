//! Every screen, at every terminal size down to nothing: none may panic.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use tui_tui::games::chess::App;
use tui_tui::games::chess::ui::PieceStyle;
use tui_tui::games::{Conn, Ctx, Seat, Table};
use tui_tui::lobby::{self, Field, Invited, Lobby};

/// Every small size, where the arithmetic is tightest, then a spread of
/// bigger ones up to a maximised window.
///
/// That is half of `cargo test` on its own, so locally it takes the very
/// smallest sizes and every third after them. CI sets `TUITUI_FULL_SWEEP` and
/// takes every one.
fn sizes() -> impl Iterator<Item = (u16, u16)> {
    let step = if std::env::var_os("TUITUI_FULL_SWEEP").is_some() {
        1
    } else {
        3
    };
    let small = move |to: u16| (0..=4).chain((5..=to).step_by(step));
    let widths = small(40).chain((41..=250).step_by(13));
    widths.flat_map(move |w| small(20).chain((21..=70).step_by(7)).map(move |h| (w, h)))
}

/// The corners, the middle, and the edges between, of a `w` by `h` screen.
fn spots(w: u16, h: u16) -> Vec<(u16, u16)> {
    let (x1, y1) = (w.saturating_sub(1), h.saturating_sub(1));
    vec![
        (0, 0),
        (x1, 0),
        (0, y1),
        (x1, y1),
        (w / 2, h / 2),
        (w / 2, 0),
        (0, h / 2),
    ]
}

fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

const GESTURES: [MouseEventKind; 4] = [
    MouseEventKind::Moved,
    MouseEventKind::Down(MouseButton::Left),
    MouseEventKind::Up(MouseButton::Left),
    MouseEventKind::Down(MouseButton::Right),
];

fn games() -> Vec<(&'static str, Table<App>)> {
    let mut out = vec![];
    for style in std::iter::successors(Some(PieceStyle::Octant), |s| Some(s.next())).take(7) {
        let mut app = Table::local(App::local());
        app.play.piece_style = style;
        app.play.game.cursor = shakmaty::Square::E2;
        app.play.game.activate(None);
        out.push(("selected", app));
    }
    let mut mated = Table::local(App::local());
    for m in ["f2f3", "e7e5", "g2g4", "d8h4"] {
        mated.play.game.play_uci(m).unwrap();
    }
    mated.play.ended = Some(
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(10))
            .unwrap(),
    );
    out.push(("mated", mated));
    let mut promo = Table::local(App::local());
    for m in [
        "a2a4", "b7b5", "a4b5", "a7a6", "b5a6", "c8b7", "a6a7", "b7c6",
    ] {
        promo.play.game.play_uci(m).unwrap();
    }
    promo.play.game.cursor = shakmaty::Square::A7;
    promo.play.game.activate(None);
    promo.play.game.cursor = shakmaty::Square::B8;
    promo.play.game.activate(None);
    out.push(("promoting", promo));
    let mut hosting = Table::new(Ctx::new(Conn::Waiting, None), App::new(Seat::Host));
    hosting.ctx.share = Some("42-tiger-marble-ocean".into());
    out.push(("hosting", hosting));
    // Talking, with more said than fits and a long message half typed.
    let (to_peer, _) = tokio::sync::mpsc::unbounded_channel();
    let mut chatting = Table::new(Ctx::new(Conn::Dialling, None), App::new(Seat::Guest));
    chatting.attach(
        tui_tui::net::Net {
            peer: iroh::SecretKey::generate().public(),
            out: to_peer,
        },
        "someone with a long name",
    );
    for i in 0..30 {
        chatting
            .ctx
            .chat
            .heard(format!("message {i} {}", "wide 象棋 ".repeat(i)));
        chatting.ctx.chat.said("ok".into());
    }
    chatting.ctx.chat.focus();
    chatting.ctx.chat.scroll = 1000;
    chatting.ctx.chat.input = "x".repeat(400);
    out.push(("chatting", chatting));
    // Looking back, and analysing with an engine that has a score in.
    let mut reviewing = Table::local(App::local());
    for m in ["e2e4", "e7e5", "g1f3"] {
        reviewing.play.game.play_uci(m).unwrap();
    }
    reviewing.play.review = Some(1);
    out.push(("reviewing", reviewing));
    #[cfg(unix)]
    out.push(("analysing", analysing()));
    // Asking whether to download an engine, with a long question to fit.
    let mut offering = Table::local(App::local());
    offering.play.offer_download = true;
    out.push(("offering a download", offering));
    let mut quitting = Table::local(App::local());
    quitting.ctx.confirm_leave = true;
    out.push(("quitting", quitting));
    out
}

/// A game being analysed, by an engine that answers with a score and a best
/// move for every position, once it has had a moment to.
#[cfg(unix)]
fn analysing() -> Table<App> {
    use std::os::unix::fs::PermissionsExt;
    // The engine's task outlives this function on a runtime kept for the
    // rest of the test run.
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let dir = std::env::temp_dir().join(format!("tuitui-{}-tiny", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("engine");
    let script = "#!/bin/sh\nwhile read -r l; do case \"$l\" in uci) echo uciok;; go*) echo \"info depth 20 score mate -3 pv e7e5\"; echo bestmove e7e5;; quit) exit;; esac; done\n";
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let runtime = RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().unwrap());
    let _entered = runtime.enter();
    let mut t = Table::local(App::local());
    t.play.game.play_uci("e2e4").unwrap();
    t.play.engine = Some(tui_tui::games::chess::engine::Engine::start(&path, None).unwrap());
    t.play.feed_engine();
    for _ in 0..300 {
        if t.play.best_move().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(t.play.best_move().is_some(), "the engine answered");
    t
}

#[test]
fn games_draw_at_any_size() {
    for (label, mut app) in games() {
        for (w, h) in sizes() {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            app.set_area(Rect::new(0, 0, w, h));
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                t.draw(|f| app.draw(f)).unwrap();
            }));
            assert!(r.is_ok(), "{label} at {w}x{h}");
        }
    }
}

#[test]
fn lobbies_draw_at_any_size() {
    let mut plain = Lobby::new();
    plain.name = "ace".into();
    let mut invited = Lobby::new();
    invited.invite = Some(Invited {
        name: "a very long name".into(),
        game: tui_tui::games::chess::KIND,
    });
    let mut typing = Lobby::new();
    typing.editing = Some(Field::Code);
    typing.input = "42-tiger-mar".into();
    let mut naming = Lobby::new();
    naming.editing = Some(Field::Name);
    naming.selected = naming.rows().len() - 2;
    naming.name_input = "someone".into();
    for (label, lobby) in [
        ("plain", plain),
        ("invited", invited),
        ("typing", typing),
        ("naming", naming),
    ] {
        for (w, h) in sizes() {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                t.draw(|f| lobby::draw_lobby(f, &lobby)).unwrap();
            }));
            assert!(r.is_ok(), "{label} at {w}x{h}");
        }
    }
}

#[test]
fn clicks_land_safely_at_any_size() {
    for (w, h) in sizes() {
        // Each game takes the whole run of clicks, as a player's would.
        for (label, mut app) in games() {
            app.set_area(Rect::new(0, 0, w, h));
            for at in spots(w, h) {
                for kind in GESTURES {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        app.on_mouse(mouse(kind, at));
                    }));
                    assert!(r.is_ok(), "{label}: {kind:?} at {at:?} on {w}x{h}");
                }
            }
        }
        let mut lobby = Lobby::new();
        lobby.area = Rect::new(0, 0, w, h);
        for at in spots(w, h) {
            for kind in GESTURES {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    lobby.on_mouse(mouse(kind, at));
                }));
                assert!(r.is_ok(), "lobby: {kind:?} at {at:?} on {w}x{h}");
            }
        }
    }
}

#[test]
fn thumbnails_draw_at_any_size_and_stay_inside() {
    use ratatui::buffer::Buffer;
    use tui_tui::games::Kind;
    let screen = Rect::new(0, 0, 60, 20);
    for &kind in Kind::ALL {
        for (w, h) in (0..=40).flat_map(|w| (0..=10).map(move |h| (w, h))) {
            let area = Rect::new(5, 3, w, h);
            let mut buf = Buffer::empty(screen);
            kind.thumb(&mut buf, area);
            for y in 0..screen.height {
                for x in 0..screen.width {
                    if !area.contains((x, y).into()) {
                        assert_eq!(buf[(x, y)].symbol(), " ", "{kind:?} at {w}x{h}: ({x}, {y})");
                        assert_eq!(buf[(x, y)].bg, ratatui::style::Color::Reset);
                    }
                }
            }
        }
        // Up against the edge of the screen, and past it.
        let mut buf = Buffer::empty(screen);
        kind.thumb(&mut buf, Rect::new(50, 15, 30, 10));
    }
}
