//! Chess: the rules ([`rules`]), the state and what keys mean ([`app`]), the
//! board ([`ui`], with pieces drawn by [`canvas`]), and what the two sides
//! say to each other ([`protocol`]).
//!
//! This is the whole of chess's connection to the rest of the program: a
//! [`Descriptor`] for the registry, and [`Play`] for the table.

pub mod analysis;
pub mod app;
pub mod canvas;
pub mod download;
pub mod engine;
pub mod protocol;
pub mod rules;
pub mod ui;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

pub use app::App;
pub use protocol::GAME;

use super::{Ctx, Descriptor, Handled, Kind, Play, Seat, Table};

pub const DESCRIPTOR: Descriptor = Descriptor {
    name: "Chess",
    blurb: "two players, one board",
    wire: GAME,
    alone: false,
    thumb: ui::thumb,
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

    fn draw(&self, f: &mut Frame, ctx: &Ctx) {
        ui::draw(f, self, ctx);
    }

    fn chat_area(&self, ctx: &Ctx) -> Option<Rect> {
        ui::Geometry::for_game(ctx.area, ctx, self).chat
    }

    fn on_wake(&mut self, ctx: &mut Ctx) {
        App::on_wake(self, ctx);
    }

    fn in_play(&self) -> bool {
        App::in_play(self)
    }

    fn is_animating(&self) -> bool {
        App::is_animating(self)
    }
}
