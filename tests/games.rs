//! The registry of games, and chess's messages.

use std::collections::HashSet;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_tui::games::chess::protocol::Msg;
use tui_tui::games::chess::{self, App};
use tui_tui::games::{Conn, Ctx, Kind, Leave, Seat, Table};
use tui_tui::net::NetEvent;
use tui_tui::session::{self, Progress};

#[test]
fn every_game_is_known_by_what_it_says_on_the_wire() {
    for &kind in Kind::ALL {
        assert_eq!(Kind::from_wire(kind.wire()), Some(kind));
    }
    assert_eq!(Kind::all_wire().len(), Kind::ALL.len());
    let stranger = session::Game {
        name: "go",
        version: 1,
    };
    assert_eq!(Kind::from_wire(stranger), None);
    let newer = session::Game {
        version: chess::GAME.version + 1,
        ..chess::GAME
    };
    assert_eq!(Kind::from_wire(newer), None, "a version we cannot speak");
}

/// The registry is where a second game gets added, so it is where two games
/// could collide: the same name in the lobby, or the same id on the wire.
#[test]
fn no_two_games_share_a_name_or_a_wire_id() {
    let names: HashSet<_> = Kind::ALL.iter().map(|k| k.name().to_lowercase()).collect();
    let wires: HashSet<_> = Kind::ALL.iter().map(|k| k.wire().name).collect();
    assert_eq!(names.len(), Kind::ALL.len(), "two games share a name");
    assert_eq!(wires.len(), Kind::ALL.len(), "two games share a wire id");
}

#[test]
fn every_game_can_be_named_on_the_command_line() {
    for &kind in Kind::ALL {
        let name = kind.name();
        assert!(
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()),
            "{name:?} must be one plain word to type"
        );
        assert_eq!(Kind::from_name(&name.to_lowercase()), Some(kind));
        assert_eq!(Kind::from_name(&name.to_uppercase()), Some(kind));
    }
    assert_eq!(Kind::from_name("draughts"), None);
    assert_eq!(Kind::DEFAULT, Kind::ALL[0]);
}

#[test]
fn every_game_starts_as_itself_at_every_seat() {
    for &kind in Kind::ALL {
        for seat in [Seat::Local, Seat::Host, Seat::Guest] {
            let table = kind.start(seat, Ctx::local());
            assert_eq!(table.kind(), kind, "{kind:?} at {seat:?}");
            assert_eq!(table.leaving(), None);
        }
    }
}

#[test]
fn a_game_with_a_bot_starts_one_as_itself() {
    for &kind in Kind::ALL {
        match kind.start_bot(Ctx::local()) {
            Some(table) => {
                assert!(kind.has_bot());
                assert_eq!(table.kind(), kind);
                // Picking the strength first: nothing to walk out on yet.
                assert!(!table.play.in_play(), "{kind:?}");
            }
            None => assert!(!kind.has_bot(), "{kind:?}"),
        }
    }
}

#[test]
fn the_seat_decides_the_side() {
    let host = App::new(Seat::Host);
    let guest = App::new(Seat::Guest);
    let local = App::new(Seat::Local);
    assert_eq!(host.me, Some(shakmaty::Color::White));
    assert_eq!(guest.me, Some(shakmaty::Color::Black));
    assert!(guest.flipped, "black sits behind black's pieces");
    assert_eq!(local.me, None);
}

#[test]
fn chess_messages_survive_the_trip() {
    for msg in [Msg::Move("e2e4".into()), Msg::Resign, Msg::Draw] {
        assert_eq!(Msg::parse(&msg.line()), Some(msg.clone()), "{msg:?}");
    }
}

#[test]
fn chess_messages_are_matched_exactly() {
    for line in [
        "castle-sideways",
        "resign please",
        "draw now",
        "move",
        "move e2e4 e7e5",
        "Move e2e4",
        " resign",
    ] {
        assert_eq!(Msg::parse(line), None, "{line:?}");
    }
}

#[test]
fn progress_moves_the_connection_along() {
    let mut table = chess::KIND.start(Seat::Host, Ctx::new(Conn::Publishing, None));
    table.on_net(NetEvent::Progress(Progress::Listed));
    assert_eq!(table.ctx.conn, Conn::Waiting);
    table.on_net(NetEvent::Progress(Progress::Declined));
    assert_eq!(
        table.ctx.note.as_deref(),
        Some("someone joined who cannot play chess")
    );
    table.on_net(NetEvent::Disconnected("gone".into()));
    assert_eq!(table.ctx.conn, Conn::Lost("gone".into()));
}

#[test]
fn a_game_behind_the_trait_leaves_like_any_other() {
    let press = |t: &mut Table<dyn tui_tui::games::Play>, code, mods| {
        t.on_key(KeyEvent::new(code, mods));
    };
    let mut table = chess::KIND.start(Seat::Local, Ctx::local());
    press(&mut table, KeyCode::Char('q'), KeyModifiers::NONE);
    press(&mut table, KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(table.leaving(), Some(Leave::Lobby));

    let mut table = chess::KIND.start(Seat::Local, Ctx::local());
    press(&mut table, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(table.leaving(), Some(Leave::Exit));
}
