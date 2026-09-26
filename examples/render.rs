//! Renders the UI to an in-memory terminal and prints it, so the layout can be
//! checked without a human at a keyboard. `cargo run --example render`

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color as Paint;
use shakmaty::{Color, Square};
use tui_tui::clipboard::Copied;
use tui_tui::games::chess::App;
use tui_tui::games::chess::ui::{Geometry, PieceStyle};
use tui_tui::games::{Conn, Table};
use tui_tui::lobby::{self, Field, Invited, Lobby};
use tui_tui::profile::Contact;

fn dump(label: &str, app: &Table<App>, w: u16, h: u16) {
    print_frame(label, w, h, |f| app.draw(f));
}

fn friend(name: &str, games: u32, last_played: u64) -> Contact {
    Contact {
        id: iroh::SecretKey::generate().public(),
        name: name.into(),
        games,
        last_played,
    }
}

fn dump_lobby(label: &str, lobby: &Lobby, w: u16, h: u16) {
    print_frame(label, w, h, |f| lobby::draw_lobby(f, lobby));
}

fn print_frame(label: &str, w: u16, h: u16, draw: impl FnOnce(&mut ratatui::Frame)) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(draw).unwrap();
    let buf = terminal.backend().buffer().clone();

    println!("\n=== {label} ({w}x{h}) ===");
    for y in 0..h {
        let row: String = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
        println!("|{}|", row.trim_end());
    }
}

/// Reads the half-block sprites back out of the rendered buffer, so what is
/// printed here is what the terminal was actually told to draw.
fn sprites(app: &Table<App>, w: u16, h: u16, squares: &[Square]) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let g = Geometry::new(Rect::new(0, 0, w, h));
    let (cw, ch) = g.cell;

    let ink = |c: Paint| match c {
        Paint::Rgb(242, 239, 232) | Paint::Rgb(40, 36, 33) => 'O',
        Paint::Rgb(46, 40, 35) | Paint::Rgb(201, 194, 182) => '#',
        _ => '.',
    };

    println!("\n=== sprites at {cw}x{ch} ({w}x{h}) ===");
    let mut grids: Vec<Vec<String>> = Vec::new();
    for sq in squares {
        let col = u16::from(sq.file() as u8);
        let row = 7 - u16::from(sq.rank() as u8);
        let mut rows = Vec::new();
        for sub in 0..ch {
            let (mut top, mut bottom) = (String::new(), String::new());
            for x in 0..cw {
                let cell = &buf[(g.grid.x + col * cw + x, g.grid.y + row * ch + sub)];
                if cell.symbol() == "▀" {
                    top.push(ink(cell.fg));
                    bottom.push(ink(cell.bg));
                } else {
                    top.push('.');
                    bottom.push('.');
                }
            }
            rows.push(top);
            rows.push(bottom);
        }
        grids.push(rows);
    }
    for i in 0..grids[0].len() {
        let line: Vec<&str> = grids.iter().map(|gr| gr[i].as_str()).collect();
        println!("  {}", line.join("  "));
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one screen after another, in the order they are printed"
)]
fn main() {
    let mut app = Table::local(App::local());
    for m in [
        "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "g8f6", "e1g1", "f6e4",
    ] {
        app.play.game.play_uci(m).unwrap();
    }
    app.play.game.cursor = Square::D2;
    app.play.game.activate(None);

    let back = [
        Square::A1,
        Square::B1,
        Square::C1,
        Square::D1,
        Square::E1,
        Square::F1,
        Square::A2,
    ];
    sprites(&Table::local(App::local()), 120, 40, &back);
    sprites(&Table::local(App::local()), 100, 30, &back);

    let mut lettered = Table::local(App::local());
    lettered.play.piece_style = PieceStyle::BigLetter;
    sprites(&lettered, 120, 40, &back);

    // The plain-text dumps only make sense with character pieces.
    app.play.piece_style = PieceStyle::Art;
    dump("big terminal", &app, 120, 40);
    dump("classic 80x24", &app, 80, 24);

    let mut hosting = Table::local(App::local());
    hosting.play.piece_style = PieceStyle::Art;
    hosting.play.me = Some(Color::White);
    hosting.ctx.conn = Conn::Waiting;
    hosting.ctx.share = Some("42-tiger-marble-ocean".into());
    hosting.ctx.copied = Some(Copied::Terminal);
    dump("hosting, waiting", &hosting, 100, 30);

    // Someone to talk to. Nothing is listening on the other end of the
    // channel, which is fine for a picture.
    let (out, _peer_hears) = tokio::sync::mpsc::unbounded_channel();
    let net = tui_tui::net::Net {
        peer: iroh::SecretKey::generate().public(),
        out,
    };
    let mut chatting = Table::local(App::local());
    chatting.play.piece_style = PieceStyle::Art;
    chatting.play.me = Some(Color::White);
    chatting.attach(net, "bob");
    for m in ["e2e4", "e7e5", "g1f3"] {
        chatting.play.game.play_uci(m).unwrap();
    }
    chatting
        .ctx
        .chat
        .heard("good luck! been a while since I played the italian".into());
    chatting.ctx.chat.said("you too, go easy on me".into());
    chatting.ctx.chat.heard("no promises".into());
    chatting.ctx.chat.heard("your move btw".into());
    chatting.ctx.chat.focus();
    chatting.ctx.chat.input = "thinking about it".into();
    dump("chatting", &chatting, 140, 40);
    dump("chatting, classic 80x24", &chatting, 80, 24);

    let mut lobby = Lobby::new();
    lobby.name = "ace".into();
    dump_lobby("lobby, first run", &lobby, 80, 30);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    lobby.set_friends(vec![
        friend("alice", 3, now - 3 * 3600),
        friend("bob", 1, now - 30 * 3600),
        friend("a very long name indeed, really", 12, now - 20 * 86400),
    ]);
    lobby.selected = 2;
    dump_lobby("lobby with friends", &lobby, 80, 30);
    lobby.game = 1;
    lobby.selected = 1;
    dump_lobby("lobby on wordle", &lobby, 100, 36);
    dump_lobby("lobby, classic 80x24", &lobby, 80, 24);
    dump_lobby("lobby, narrow", &lobby, 50, 24);
    dump_lobby("lobby, short", &lobby, 80, 14);
    dump_lobby("lobby, tiny", &lobby, 36, 10);
    lobby.game = 0;
    lobby.invite = Some(Invited {
        name: "alice".into(),
        game: tui_tui::games::chess::KIND,
    });
    dump_lobby("an invite", &lobby, 80, 30);
    lobby.invite = None;

    lobby.set_friends(vec![]);
    lobby.editing = Some(Field::Code);
    lobby.selected = 0;
    lobby.input = "42-tiger-mar".into();
    dump_lobby("lobby, typing a code", &lobby, 80, 24);
    lobby.input = "42-tiger-xylo".into();
    dump_lobby("lobby, a word that is not one", &lobby, 60, 20);

    let mut promo = Table::local(App::local());
    promo.play.piece_style = PieceStyle::Art;
    for m in [
        "d2d4", "e7e5", "d4e5", "d7d5", "e5d6", "g8f6", "d6c7", "a7a6",
    ] {
        promo.play.game.play_uci(m).unwrap();
    }
    promo.play.game.cursor = Square::C7;
    promo.play.game.activate(None);
    promo.play.game.cursor = Square::B8;
    promo.play.game.activate(None);
    dump("promotion prompt", &promo, 100, 30);
}
