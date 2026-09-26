//! The short code two players share: `42-tiger-marble-ocean`.
//!
//! A number and three words from the BIP39 English list, which has 2048 words
//! and no two of them starting with the same four letters. That makes about
//! 40 bits, enough that nobody can walk the code space on the DHT looking for
//! live games, while staying short enough to read out over a call.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

const WORDLIST: &str = include_str!("words.txt");

static WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| WORDLIST.lines().collect());

/// Codes are numbered from 0 to 99.
const NUMBERS: u8 = 100;
const WORD_COUNT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    number: u8,
    words: [u16; WORD_COUNT],
}

impl Code {
    /// A fresh code from the operating system's random source.
    ///
    /// # Panics
    ///
    /// If the operating system has no random source to draw from.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        loop {
            getrandom::fill(&mut bytes).expect("the OS has no random source");
            // 200 is the largest multiple of 100 a byte holds; anything over
            // it would favour the low numbers, so draw again.
            if bytes[0] < 2 * NUMBERS {
                break;
            }
        }
        // The list is exactly 2^11 long, so eleven bits pick a word evenly.
        let word = |i: usize| u16::from_le_bytes([bytes[1 + 2 * i], bytes[2 + 2 * i]]) & 0x7ff;
        Self {
            number: bytes[0] % NUMBERS,
            words: [word(0), word(1), word(2)],
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.number)?;
        for &w in &self.words {
            write!(f, "-{}", WORDS[w as usize])?;
        }
        Ok(())
    }
}

/// Why typed text is not a code, worded for the person who typed it.
#[derive(Debug, PartialEq, Eq)]
pub enum CodeError {
    Shape,
    Number(String),
    Word(String),
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeError::Shape => write!(f, "a code looks like 42-tiger-marble-ocean"),
            CodeError::Number(n) => write!(f, "{n:?} should be a number from 0 to 99"),
            CodeError::Word(w) => write!(f, "{w:?} is not one of the code words"),
        }
    }
}

impl std::error::Error for CodeError {}

impl FromStr for Code {
    type Err = CodeError;

    /// Forgiving about the things people change when they retype a code:
    /// case, spaces for dashes, stray whitespace, and a word cut short after
    /// its first four letters.
    fn from_str(s: &str) -> Result<Self, CodeError> {
        let lower = s.trim().to_lowercase();
        let parts: Vec<&str> = lower
            .split(|c: char| c == '-' || c == '_' || c == '.' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .collect();
        let [number, rest @ ..] = parts.as_slice() else {
            return Err(CodeError::Shape);
        };
        if rest.len() != WORD_COUNT {
            return Err(CodeError::Shape);
        }
        let number = number
            .parse::<u8>()
            .ok()
            .filter(|n| *n < NUMBERS)
            .ok_or_else(|| CodeError::Number(number.to_string()))?;
        let mut words = [0; WORD_COUNT];
        for (slot, part) in words.iter_mut().zip(rest) {
            *slot = lookup(part).ok_or_else(|| CodeError::Word(part.to_string()))?;
        }
        Ok(Self { number, words })
    }
}

/// The one code word `partial` can still become, if there is only one.
pub fn complete(partial: &str) -> Option<&'static str> {
    if partial.is_empty() {
        return None;
    }
    let mut matches = WORDS.iter().filter(|w| w.starts_with(partial));
    let only = matches.next()?;
    matches.next().is_none().then_some(*only)
}

/// Whether some code word starts with `partial`.
pub fn is_prefix(partial: &str) -> bool {
    WORDS.iter().any(|w| w.starts_with(partial))
}

/// Whether `part` names a code word, the same way parsing a code decides it.
#[must_use]
pub fn is_word(part: &str) -> bool {
    lookup(part).is_some()
}

fn lookup(part: &str) -> Option<u16> {
    let exact = WORDS.binary_search(&part).ok();
    // Four letters are enough to name a word, so accept one cut short as long
    // as at least that much of it is left.
    let prefix = || {
        (part.len() >= 4)
            .then(|| WORDS.iter().position(|w| w.starts_with(part)))
            .flatten()
    };
    exact.or_else(prefix).and_then(|i| u16::try_from(i).ok())
}
