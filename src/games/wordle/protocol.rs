//! What two Wordle players say to each other once they are connected.
//!
//! Both race to guess the same word, so both sides need to know it, and
//! neither should get to choose it. Before each round each side draws a
//! secret, and says only its hash — `round easy <hash>` — which is both an
//! offer to play and a promise. Once the two offers agree on the mode, each
//! side reveals its secret with `seed <secret>`, checked against the hash it
//! promised, and the word comes from both secrets together. Whoever reveals
//! second has already promised, so cannot pick a secret to steer the word.
//!
//! Each guess goes over as the word itself, with how long into the round it
//! came: `guess crane 8412`. The other side marks it against the answer
//! itself rather than taking anyone's word for the colours.

use crate::session::Game;

use super::words::{self, Word};

pub const GAME: Game = Game {
    name: "wordle",
    version: 1,
};

/// A secret, or the hash that promises it.
pub type Bytes = [u8; 32];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    /// Ready for a round in this mode, promising the secret behind `commit`.
    Round { hard: bool, commit: Bytes },
    /// The secret promised.
    Seed(Bytes),
    /// A guess, made `ms` milliseconds into the round.
    Guess { word: Word, ms: u64 },
}

impl Msg {
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Msg::Round { hard, commit } => {
                let mode = if *hard { "hard" } else { "easy" };
                format!("round {mode} {}", hex(commit))
            }
            Msg::Seed(secret) => format!("seed {}", hex(secret)),
            Msg::Guess { word, ms } => {
                format!("guess {} {ms}", String::from_utf8_lossy(word))
            }
        }
    }

    /// `None` for anything unknown, which is ignored so the protocol can grow,
    /// and for anything malformed.
    #[must_use]
    pub fn parse(line: &str) -> Option<Msg> {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts[..] {
            ["round", mode, commit] => Some(Msg::Round {
                hard: match mode {
                    "hard" => true,
                    "easy" => false,
                    _ => return None,
                },
                commit: unhex(commit)?,
            }),
            ["seed", secret] => Some(Msg::Seed(unhex(secret)?)),
            // Lower case only, as it is sent: anything else is not ours.
            ["guess", word, ms] if word.bytes().all(|b| b.is_ascii_lowercase()) => {
                Some(Msg::Guess {
                    word: words::word(word)?,
                    ms: ms.parse().ok()?,
                })
            }
            _ => None,
        }
    }
}

fn hex(bytes: &Bytes) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(s: &str) -> Option<Bytes> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}
