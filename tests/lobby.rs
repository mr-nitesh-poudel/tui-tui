//! The lobby: the shelf of games, what to do with one, typing a code in,
//! friends, and invites.

use iroh::SecretKey;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use tui_tui::games::chess::App;
use tui_tui::games::{Conn, Kind, Table};
use tui_tui::lobby::{Choice, Entry, FRIENDS_SHOWN, Field, Invited, Item, Lobby, Row};
use tui_tui::lobby::{LobbyGeometry, Shelf};
use tui_tui::profile::Contact;
use tui_tui::session::Code;

fn key(lobby: &mut Lobby, code: KeyCode) -> Option<Choice> {
    lobby.on_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn typed(lobby: &mut Lobby, text: &str) -> Option<Choice> {
    text.chars().find_map(|c| key(lobby, KeyCode::Char(c)))
}

fn code(s: &str) -> Code {
    s.parse().unwrap()
}

fn index(lobby: &Lobby, item: Item) -> usize {
    lobby
        .rows()
        .iter()
        .position(|&r| r == Row::Item(item))
        .unwrap()
}

fn friend(name: &str, last_played: u64) -> Contact {
    Contact {
        id: SecretKey::generate().public(),
        name: name.into(),
        games: 1,
        last_played,
    }
}

fn joining(lobby: &Lobby) -> bool {
    lobby.editing == Some(Field::Code)
}

#[test]
fn the_rows_pick_what_is_selected() {
    let mut lobby = Lobby::new();
    assert_eq!(lobby.rows()[lobby.selected], Row::Item(Item::Host));
    assert_eq!(key(&mut lobby, KeyCode::Enter), Some(Choice::Host));

    let mut lobby = Lobby::new();
    key(&mut lobby, KeyCode::Down);
    assert_eq!(key(&mut lobby, KeyCode::Enter), Some(Choice::Local));

    // Wrapping upwards from the first lands on the last: the code box,
    // where enter starts typing rather than anything else.
    let mut lobby = Lobby::new();
    key(&mut lobby, KeyCode::Up);
    assert_eq!(lobby.rows()[lobby.selected], Row::Code);
    assert_eq!(key(&mut lobby, KeyCode::Enter), None);
    assert!(joining(&lobby));

    assert_eq!(
        key(&mut Lobby::new(), KeyCode::Char('q')),
        Some(Choice::Quit)
    );
}

#[test]
fn left_and_right_change_the_game_from_anywhere() {
    let mut lobby = Lobby::new();
    lobby.set_friends(vec![friend("alice", 0)]);
    assert_eq!(lobby.game(), Kind::DEFAULT);

    for selected in 0..lobby.rows().len() {
        lobby.selected = selected;
        for code in [KeyCode::Right, KeyCode::Left] {
            assert_eq!(key(&mut lobby, code), None, "{code:?} starts nothing");
            assert_eq!(lobby.selected, selected, "and leaves the selection be");
        }
    }
    key(&mut lobby, KeyCode::Right);
    assert_eq!(lobby.game, 1 % Kind::ALL.len());
    // Round the list and back to where it started, however long it is.
    let before = lobby.game();
    for _ in 0..Kind::ALL.len() {
        key(&mut lobby, KeyCode::Right);
    }
    assert_eq!(lobby.game(), before);
}

#[test]
fn what_is_offered_reads_the_same_for_every_game() {
    assert_eq!(Item::Host.label(), "Host a game");
    assert_eq!(Item::Local.label(), "Play solo");
}

#[test]
fn esc_backs_out_of_a_code() {
    let mut lobby = Lobby::new();
    typed(&mut lobby, "4");
    assert!(joining(&lobby));

    // In the box, q is part of a word, not a way out.
    assert_eq!(key(&mut lobby, KeyCode::Char('q')), None);
    assert_eq!(lobby.input, "4q");

    key(&mut lobby, KeyCode::Esc);
    assert!(!joining(&lobby));
}

#[test]
fn typing_a_number_from_the_menu_starts_a_code() {
    let mut lobby = Lobby::new();
    assert_eq!(typed(&mut lobby, "42 Tiger marble ocean"), None);
    assert!(joining(&lobby));
    assert_eq!(lobby.rows()[lobby.selected], Row::Code, "the box lights up");
    assert_eq!(lobby.input, "42-tiger-marble-ocean");
    assert_eq!(
        key(&mut lobby, KeyCode::Enter),
        Some(Choice::Join(code("42-tiger-marble-ocean")))
    );
}

#[test]
fn enter_does_nothing_until_the_code_is_whole() {
    let mut lobby = Lobby::new();
    typed(&mut lobby, "42-tiger-marble");
    assert_eq!(key(&mut lobby, KeyCode::Enter), None);
    assert!(joining(&lobby));
}

#[test]
fn tab_finishes_a_word_and_moves_on() {
    let mut lobby = Lobby::new();
    typed(&mut lobby, "42");
    key(&mut lobby, KeyCode::Tab);
    assert_eq!(lobby.input, "42-");

    typed(&mut lobby, "tig");
    assert_eq!(lobby.completion(), Some("tiger"));
    key(&mut lobby, KeyCode::Tab);
    assert_eq!(lobby.input, "42-tiger-");

    // "mar" could be marble, march, margin...: nothing to finish, and no
    // dash past a word that is not one yet.
    typed(&mut lobby, "mar");
    assert_eq!(lobby.completion(), None);
    key(&mut lobby, KeyCode::Tab);
    assert_eq!(lobby.input, "42-tiger-mar");
}

#[test]
fn bad_parts_are_flagged_as_you_go() {
    let entry = |text: &str| {
        let mut lobby = Lobby::new();
        typed(&mut lobby, text);
        lobby.entry()
    };
    assert_eq!(entry("4"), Entry::Typing);
    assert_eq!(entry("42-tig"), Entry::Typing);
    assert!(matches!(entry("42-xylo"), Entry::Bad(_)));
    assert!(matches!(entry("420-tiger"), Entry::Bad(_)));
    assert!(matches!(entry("42-tigr-marble"), Entry::Bad(_)));
    assert_eq!(
        entry("42-tiger-marble-ocean"),
        Entry::Ready(code("42-tiger-marble-ocean"))
    );
}

#[test]
fn ctrl_w_drops_the_last_part() {
    let mut lobby = Lobby::new();
    typed(&mut lobby, "42-tiger-marb");
    lobby.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(lobby.input, "42-tiger-");
    lobby.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(lobby.input, "42-");
}

#[test]
fn pasting_takes_the_code_or_the_whole_command() {
    for pasted in [
        "42-tiger-marble-ocean",
        "  42 Tiger Marble Ocean\n",
        "tuitui join 42-tiger-marble-ocean\n",
    ] {
        let mut lobby = Lobby::new();
        lobby.on_paste(pasted);
        assert!(joining(&lobby), "{pasted:?}");
        assert_eq!(
            lobby.entry(),
            Entry::Ready(code("42-tiger-marble-ocean")),
            "{pasted:?}"
        );
    }
}

#[test]
fn clicking_an_item_chooses_it() {
    let mut lobby = Lobby::new();
    lobby.name = "ace".into();
    lobby.set_friends(vec![friend("alice", 0)]);
    lobby.area = Rect::new(0, 0, 80, 30);
    let g = LobbyGeometry::new(lobby.area, &lobby);
    let row = |i: usize| g.rows[i];
    let at = |i: usize| (row(i).x + 2, row(i).y);
    let mouse = |kind, (column, row)| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let (host, local) = (index(&lobby, Item::Host), index(&lobby, Item::Local));

    // Hovering selects, clicking chooses.
    lobby.on_mouse(mouse(MouseEventKind::Moved, at(local)));
    assert_eq!(lobby.selected, local);
    let click = MouseEventKind::Down(MouseButton::Left);
    assert_eq!(lobby.on_mouse(mouse(click, at(host))), Some(Choice::Host));
    // The whole width of the menu is the item, not just its words.
    let edge = (row(local).right() - 1, row(local).y);
    assert_eq!(lobby.on_mouse(mouse(click, edge)), Some(Choice::Local));

    let alice = lobby.friends[0].id;
    let on_alice = lobby
        .rows()
        .iter()
        .position(|&r| r == Row::Friend(0))
        .unwrap();
    assert_eq!(
        lobby.on_mouse(mouse(click, at(on_alice))),
        Some(Choice::Challenge(alice))
    );

    // A card picks its game, and starts nothing.
    let last = Kind::ALL.len() - 1;
    let card = g.cards[last];
    assert_eq!(lobby.on_mouse(mouse(click, (card.x + 1, card.y + 1))), None);
    assert_eq!(lobby.game, last);

    // The name renames.
    assert_eq!(lobby.on_mouse(mouse(click, (g.name.x + 1, g.name.y))), None);
    assert_eq!(lobby.editing, Some(Field::Name));

    // Anywhere on the code box, border and all, starts a code.
    let code = lobby.rows().iter().position(|&r| r == Row::Code).unwrap();
    assert_eq!(g.rows[code], g.code_box);
    assert!(g.code_box.contains((g.input.x, g.input.y).into()));
    for spot in [(g.code_box.x, g.code_box.y), (g.input.x, g.input.y)] {
        lobby.editing = None;
        assert_eq!(lobby.on_mouse(mouse(click, spot)), None);
        assert!(joining(&lobby), "{spot:?}");
    }
}

#[test]
fn every_row_and_card_has_its_own_place_on_screen() {
    let mut lobby = Lobby::new();
    for (w, h) in [(80, 24), (80, 40), (50, 30), (140, 50)] {
        for friends in [0, 1, FRIENDS_SHOWN, FRIENDS_SHOWN + 3] {
            lobby.set_friends((0..friends).map(|i| friend(&format!("f{i}"), 0)).collect());
            let rows = lobby.rows();
            let g = LobbyGeometry::new(Rect::new(0, 0, w, h), &lobby);
            let at = format!("{friends} friends at {w}x{h}");
            assert_eq!(g.rows.len(), rows.len());
            assert_eq!(g.cards.len(), Kind::ALL.len());
            let placed: Vec<Rect> = g.rows.iter().chain(&g.cards).copied().collect();
            for (i, a) in placed.iter().enumerate() {
                assert!(a.height > 0, "all fit, {at}");
                for b in &placed[i + 1..] {
                    assert!(!a.intersects(*b), "{a:?} and {b:?} overlap, {at}");
                }
                assert!(a.bottom() <= g.status.y, "clear of the status bar, {at}");
                assert!(a.y >= g.header.bottom(), "clear of the header, {at}");
            }
        }
    }
}

#[test]
fn the_shelf_gives_way_before_the_rows() {
    let lobby = Lobby::new();
    let shelf = |w, h| LobbyGeometry::new(Rect::new(0, 0, w, h), &lobby).shelf;
    assert_eq!(shelf(80, 24), Shelf::Cards { thumbs: true });
    assert_eq!(shelf(80, 19), Shelf::Cards { thumbs: false });
    assert_eq!(shelf(80, 16), Shelf::Tabs);
    assert_eq!(shelf(30, 24), Shelf::Tabs);

    // The code box keeps its border longer than the pictures do, and is
    // still a line to type on when it has lost it.
    let code_box = |w, h| LobbyGeometry::new(Rect::new(0, 0, w, h), &lobby).code_box;
    assert_eq!(code_box(80, 16).height, 3);
    assert_eq!(code_box(80, 12).height, 1);
    assert_eq!(code_box(80, 9).height, 1);
}

#[test]
fn enter_on_a_friend_challenges_them_and_x_forgets() {
    let mut lobby = Lobby::new();
    lobby.set_friends(vec![friend("alice", 2), friend("bob", 1)]);
    let (alice, bob) = (lobby.friends[0].id, lobby.friends[1].id);

    // Tab goes over to the friends, then the code box, and round again;
    // shift-Tab the other way.
    for want in [Row::Friend(0), Row::Code, Row::Item(Item::Host)] {
        key(&mut lobby, KeyCode::Tab);
        assert_eq!(lobby.rows()[lobby.selected], want);
    }
    key(&mut lobby, KeyCode::BackTab);
    assert_eq!(lobby.rows()[lobby.selected], Row::Code);
    key(&mut lobby, KeyCode::Tab);
    // As do the arrows, past what there is to do.
    key(&mut lobby, KeyCode::Down);
    key(&mut lobby, KeyCode::Down);
    assert_eq!(lobby.rows()[lobby.selected], Row::Friend(0));
    assert_eq!(
        key(&mut lobby, KeyCode::Enter),
        Some(Choice::Challenge(alice))
    );

    key(&mut lobby, KeyCode::Down);
    assert_eq!(key(&mut lobby, KeyCode::Char('x')), None);
    assert_eq!(lobby.forgetting, Some(1));
    // Anything but y keeps them.
    assert_eq!(key(&mut lobby, KeyCode::Char('n')), None);
    assert_eq!(lobby.forgetting, None);
    key(&mut lobby, KeyCode::Char('x'));
    assert_eq!(
        key(&mut lobby, KeyCode::Char('y')),
        Some(Choice::Forget(bob))
    );

    // x means nothing away from a friend, and Tab skips the friends when
    // there are none.
    let mut lobby = Lobby::new();
    key(&mut lobby, KeyCode::Char('x'));
    assert_eq!(lobby.forgetting, None);
    key(&mut lobby, KeyCode::Tab);
    assert_eq!(lobby.rows()[lobby.selected], Row::Code);
    assert!(!joining(&lobby), "only enter or typing starts a code");
}

#[test]
fn the_selection_stays_put_when_friends_change() {
    let mut lobby = Lobby::new();
    lobby.set_friends(vec![friend("alice", 0)]);
    lobby.selected = index(&lobby, Item::Local);
    lobby.set_friends(vec![]);
    assert_eq!(lobby.rows()[lobby.selected], Row::Item(Item::Local));

    lobby.selected = index(&lobby, Item::Local) + 1;
    lobby.set_friends(vec![friend("alice", 0), friend("bob", 0)]);
    assert_eq!(lobby.rows()[lobby.selected], Row::Code);

    lobby.selected = 3;
    lobby.set_friends(vec![friend("alice", 0)]);
    assert!(lobby.selected < lobby.rows().len());
}

#[test]
fn renaming_edits_in_place() {
    let mut lobby = Lobby::new();
    lobby.name = "ace".into();
    assert_eq!(key(&mut lobby, KeyCode::Char('n')), None);
    assert_eq!(lobby.editing, Some(Field::Name));
    assert_eq!(lobby.name_input, "ace");

    // Letters that mean something in the menu are just letters here.
    typed(&mut lobby, " q jk");
    assert_eq!(
        key(&mut lobby, KeyCode::Enter),
        Some(Choice::Rename("ace q jk".into()))
    );
    assert_eq!(lobby.editing, None);

    // Esc keeps the old name.
    key(&mut lobby, KeyCode::Char('n'));
    typed(&mut lobby, "zzz");
    assert_eq!(key(&mut lobby, KeyCode::Esc), None);
    assert_eq!(lobby.editing, None);
}

fn alice_invites() -> Invited {
    Invited {
        name: "alice".into(),
        game: tui_tui::games::chess::KIND,
    }
}

#[test]
fn an_invite_is_answered_before_anything_else() {
    let mut lobby = Lobby::new();
    lobby.invite = Some(alice_invites());
    // Menu keys do nothing while it is up.
    assert_eq!(key(&mut lobby, KeyCode::Char('q')), None);
    assert_eq!(key(&mut lobby, KeyCode::Down), None);
    assert_eq!(
        key(&mut lobby, KeyCode::Char('y')),
        Some(Choice::AcceptInvite)
    );
    assert_eq!(lobby.invite, None);

    lobby.invite = Some(alice_invites());
    assert_eq!(key(&mut lobby, KeyCode::Esc), Some(Choice::DeclineInvite));
    assert_eq!(lobby.invite, None);
}

#[test]
fn c_copies_the_code_only_while_it_is_needed() {
    let mut app = Table::local(App::local());
    app.ctx.share = Some("42-tiger-marble-ocean".into());
    let c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);

    app.on_key(c);
    assert_eq!(app.ctx.copy_request, None, "hot-seat has nothing to share");

    app.ctx.conn = Conn::Waiting;
    app.on_key(c);
    assert_eq!(
        app.ctx.copy_request.as_deref(),
        Some("42-tiger-marble-ocean")
    );

    app.ctx.copy_request = None;
    app.ctx.conn = Conn::Playing;
    app.on_key(c);
    assert_eq!(app.ctx.copy_request, None, "the opponent is already in");
}
