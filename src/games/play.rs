//! What a game has to be able to do.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

use super::{Ctx, Kind};

/// Whether a game made use of a key. Keys it did not use go on to the
/// [`Table`](super::Table), so a game only has to know its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    Used,
    Unused,
}

/// A game in progress: its rules, its screen, and its half of the
/// conversation with the opponent.
///
/// The table calls these; a game never talks to the network or the terminal
/// except through the [`Ctx`] it is handed.
pub trait Play {
    fn kind(&self) -> Kind;

    /// A key the player pressed. Only presses arrive, never releases or
    /// repeats, and never Ctrl-C, or anything while the table is asking the
    /// player a question of its own.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx) -> Handled;

    /// A mouse event. [`Ctx::area`] is the terminal's size, for working out
    /// what was clicked.
    fn on_mouse(&mut self, ev: MouseEvent, ctx: &mut Ctx);

    /// One line from the opponent, without its newline. Untrusted: it may be
    /// malformed, out of turn or hostile, and must be checked against the
    /// game's own state before it changes anything.
    fn on_line(&mut self, line: &str, ctx: &mut Ctx);

    /// The opponent is in, and anything sent now reaches them. Called once,
    /// before any of their lines arrive; never in hot-seat.
    fn on_connect(&mut self, ctx: &mut Ctx) {
        let _ = ctx;
    }

    /// The connection could not be made, or went. [`Ctx::conn`] says why.
    fn on_disconnect(&mut self, ctx: &mut Ctx) {
        let _ = ctx;
    }

    /// The whole screen. [`chrome`](super::chrome) has the parts every
    /// game's screen shares.
    fn draw(&self, f: &mut Frame, ctx: &Ctx);

    /// Whether leaving now would walk out on a game still being played. The
    /// table asks the player first if so; once a game is decided it lets them
    /// go straight away.
    fn in_play(&self) -> bool;

    /// Where [`draw`](Play::draw) puts the chat panel, if it shows one, so
    /// the table can send it the clicks that land there. A game that draws
    /// no chat leaves this alone, and its players cannot talk.
    fn chat_area(&self, ctx: &Ctx) -> Option<Rect> {
        let _ = ctx;
        None
    }

    /// Background work the game started, through [`Ctx::waker`], has news.
    /// The screen is drawn straight after; this is the game's chance to act
    /// on the news first.
    fn on_wake(&mut self, ctx: &mut Ctx) {
        let _ = ctx;
    }

    /// Whether to redraw on a timer, rather than only when something happens.
    fn is_animating(&self) -> bool {
        false
    }
}
