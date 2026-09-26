//! Wordle: the words and how a guess is marked ([`words`]), the match and
//! what keys mean ([`app`]), the screen ([`ui`]), and what the two sides say
//! to each other ([`protocol`]).
//!
//! Two players race to find the same word, each seeing only the colours of
//! the other's guesses. At one keyboard it is a game for one.

pub mod app;
pub mod font;
pub mod protocol;
pub mod ui;
pub mod words;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

pub use app::App;
pub use protocol::GAME;

use super::{Ctx, Descriptor, Handled, Kind, Play, Seat, Table};

pub const DESCRIPTOR: Descriptor = Descriptor {
    name: "Wordle",
    wire: GAME,
    alone: true,
    start,
};

pub const KIND: Kind = Kind::new(&DESCRIPTOR);

fn start(seat: Seat, ctx: Ctx) -> Box<Table<dyn Play>> {
    Box::new(Table::new(ctx, App::new(seat)))
}

impl Play for App {
    fn kind(&self) -> Kind {
        KIND
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx) -> Handled {
        App::on_key(self, key, ctx)
    }

    fn on_mouse(&mut self, ev: MouseEvent, ctx: &mut Ctx) {
        App::on_mouse(self, ev, ctx);
    }

    fn on_line(&mut self, line: &str, ctx: &mut Ctx) {
        App::on_line(self, line, ctx);
    }

    fn on_connect(&mut self, ctx: &mut Ctx) {
        App::on_connect(self, ctx);
    }

    fn on_disconnect(&mut self, ctx: &mut Ctx) {
        App::on_disconnect(self, ctx);
    }

    fn draw(&self, f: &mut Frame, ctx: &Ctx) {
        ui::draw(f, self, ctx);
    }

    fn chat_area(&self, ctx: &Ctx) -> Option<Rect> {
        ui::Geometry::of(ctx.area, ctx, self).chat
    }

    fn in_play(&self) -> bool {
        App::in_play(self)
    }

    fn is_animating(&self) -> bool {
        App::is_animating(self)
    }
}
