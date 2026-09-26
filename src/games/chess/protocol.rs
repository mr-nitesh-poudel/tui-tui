//! What two chess players say to each other once they are connected: a UCI
//! move per line, plus a few control words.

use crate::session::Game;

pub const GAME: Game = Game {
    name: "chess",
    version: 1,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    Move(String),
    Resign,
    /// Offers a draw, or accepts the one on offer.
    Draw,
}

impl Msg {
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Msg::Move(uci) => format!("move {uci}"),
            Msg::Resign => "resign".into(),
            Msg::Draw => "draw".into(),
        }
    }

    /// `None` for anything unknown, which is ignored so the protocol can grow,
    /// and for anything malformed. The match is exact: `resign` with words
    /// after it is not a resignation, and a move is one word, not zero or two.
    #[must_use]
    pub fn parse(line: &str) -> Option<Msg> {
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match (word, rest) {
            ("move", uci) if !uci.is_empty() && !uci.contains(' ') => Some(Msg::Move(uci.into())),
            ("resign", "") => Some(Msg::Resign),
            ("draw", "") => Some(Msg::Draw),
            _ => None,
        }
    }
}
