//! The keystrokes in demo.tape really do play fool's mate.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use shakmaty::Position;
use tui_tui::games::Table;
use tui_tui::games::chess::App;

#[test]
fn the_tape_ends_in_checkmate() {
    let mut app = Table::local(App::local());
    let press = |app: &mut Table<App>, code| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    let up = |app: &mut Table<App>, n| (0..n).for_each(|_| press(app, KeyCode::Up));
    let down = |app: &mut Table<App>, n| (0..n).for_each(|_| press(app, KeyCode::Down));
    let left = |app: &mut Table<App>, n| (0..n).for_each(|_| press(app, KeyCode::Left));
    let right = |app: &mut Table<App>, n| (0..n).for_each(|_| press(app, KeyCode::Right));
    let enter = |app: &mut Table<App>| press(app, KeyCode::Enter);

    // 1. f3 — the cursor starts on e2
    right(&mut app, 1);
    enter(&mut app);
    up(&mut app, 1);
    enter(&mut app);
    // 1... e5
    left(&mut app, 1);
    up(&mut app, 4);
    enter(&mut app);
    down(&mut app, 2);
    enter(&mut app);
    // 2. g4
    right(&mut app, 2);
    down(&mut app, 3);
    enter(&mut app);
    up(&mut app, 2);
    enter(&mut app);
    // 2... Qh4#
    left(&mut app, 3);
    up(&mut app, 4);
    enter(&mut app);
    right(&mut app, 4);
    down(&mut app, 4);
    enter(&mut app);

    let moves: Vec<String> = app.play.game.history.clone();
    assert!(app.play.game.pos.is_checkmate(), "not mate after {moves:?}");
}
