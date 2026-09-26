//! The games, and the table every one of them is played at.
//!
//! A game is a module under here with a type that implements [`Play`]: its
//! own rules, its own screen, and the messages its two sides send each other.
//! Everything else — pairing, invites, the lobby, leaving, the mouse, the
//! share code, the connection — belongs to the [`Table`] the game is played
//! at, and is the same for every game.
//!
//! # Adding a game
//!
//! 1. A module here, say `games/go/`, with a type that implements [`Play`].
//! 2. A [`Descriptor`] in it: the game's name, a line about it, its wire id
//!    and version, a thumbnail for its card in the lobby, and how to start
//!    one at a [`Seat`].
//! 3. One line in [`Kind::ALL`].
//!
//! The lobby, the command line, pairing and invites pick it up from there.
//! `chess` is the worked example.
//!
//! # What a game can rely on, and what it must do
//!
//! - Keys the game has no use for go back to the table, which knows `q` and
//!   `esc` (leave, asking first if [`Play::in_play`]), `m` (hand the mouse
//!   back to the terminal) and `c` (copy the share code). Ctrl-C never reaches
//!   a game: it always quits.
//! - Lines from the opponent arrive through [`Play::on_line`], each at most
//!   [`MAX_LINE`](crate::session::lines::MAX_LINE) bytes of UTF-8, but
//!   otherwise exactly as sent: possibly malformed, out of turn, or hostile.
//!   There is no referee, so the game checks every one against its own state
//!   and ignores whatever does not fit. Nothing a peer sends may panic it.
//! - Its own lines go out through [`Ctx::send`], which drops them until
//!   there is an opponent. [`Play::on_connect`] says when one arrives, before
//!   any of their lines, and [`Play::on_disconnect`] when they go.
//! - Work it does in the background calls [`Ctx::waker`] when it has
//!   something new to show, and the screen is drawn again. Lines starting with the
//!   word `chat` are the table's: it takes them before the game sees them, so
//!   a game must not send any of its own.
//! - Chat is the table's too, but where it goes on the screen is the game's
//!   choice: it draws the panel with [`chat::draw`] and says where with
//!   [`Play::chat_area`], so the table can send clicks there. While the
//!   player is typing, no keys reach the game.
//! - [`chrome`] draws what every game's screen shares: the connection status
//!   and the footer, with the table's own questions in it.

pub mod chat;
pub mod chess;
pub mod chrome;
mod play;
mod table;
pub mod wordle;

pub use play::{Handled, Play};
pub use table::{Conn, Ctx, Leave, Table, Waker};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::session;

/// Everything the rest of the program needs to know about one game.
pub struct Descriptor {
    /// Shown in the lobby, and typed on the command line in lower case, so
    /// one word.
    pub name: &'static str,
    /// A few words under the name on the game's card in the lobby.
    pub blurb: &'static str,
    /// What the two sides agree on when pairing. Bump the version whenever
    /// the game's messages change in a way an older build would misread.
    pub wire: session::Game,
    /// Played by one player at [`Seat::Local`], rather than two sharing the
    /// keyboard.
    pub alone: bool,
    /// Draws a picture of the game into the area given, for its card in the
    /// lobby. It must cope with any size, down to nothing, and stay inside.
    pub thumb: fn(&mut Buffer, Rect),
    /// A new game, sat at `seat`, at a table with this connection.
    pub start: fn(Seat, Ctx) -> Box<Table<dyn Play>>,
}

/// One of the games there is to play.
#[derive(Clone, Copy)]
pub struct Kind(&'static Descriptor);

impl Kind {
    /// Every game this build can play, in the order the lobby offers them.
    /// The first is the default.
    pub const ALL: &'static [Kind] = &[chess::KIND, wordle::KIND];

    /// The game played when none is named. Evaluated when the program is
    /// built, so an empty [`Kind::ALL`] fails to compile rather than panics.
    pub const DEFAULT: Kind = Kind::ALL[0];

    #[must_use]
    pub const fn new(descriptor: &'static Descriptor) -> Self {
        Self(descriptor)
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        self.0.name
    }

    #[must_use]
    pub fn blurb(self) -> &'static str {
        self.0.blurb
    }

    /// Draws the game's picture into `area` of `buf`.
    pub fn thumb(self, buf: &mut Buffer, area: Rect) {
        let area = area.intersection(buf.area);
        if !area.is_empty() {
            (self.0.thumb)(buf, area);
        }
    }

    /// Whether playing locally is one player on their own, rather than two
    /// taking turns.
    #[must_use]
    pub fn alone(self) -> bool {
        self.0.alone
    }

    /// The name and protocol version the two sides agree on when pairing.
    #[must_use]
    pub fn wire(self) -> session::Game {
        self.0.wire
    }

    /// The game a peer offered, if this build can play it.
    #[must_use]
    pub fn from_wire(game: session::Game) -> Option<Kind> {
        Kind::ALL.iter().copied().find(|k| k.wire() == game)
    }

    /// A game named on the command line, in any case.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL
            .iter()
            .copied()
            .find(|k| k.name().eq_ignore_ascii_case(name))
    }

    /// What pairing offers and accepts: every game this build can play.
    #[must_use]
    pub fn all_wire() -> Vec<session::Game> {
        Kind::ALL.iter().map(|k| k.wire()).collect()
    }

    /// A new game of this kind, sat at `seat`.
    pub fn start(self, seat: Seat, ctx: Ctx) -> Box<Table<dyn Play>> {
        (self.0.start)(seat, ctx)
    }
}

/// Two kinds are the same game if they say the same thing on the wire.
impl PartialEq for Kind {
    fn eq(&self, other: &Self) -> bool {
        self.wire() == other.wire()
    }
}

impl Eq for Kind {}

impl std::fmt::Debug for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Where a player sits, which decides who goes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    /// Both players at this keyboard, or the one player of a game played
    /// [`alone`](Kind::alone).
    Local,
    /// The one who hosted, or was invited.
    Host,
    /// The one who joined, or invited.
    Guest,
}
