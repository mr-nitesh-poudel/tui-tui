//! The words, and how a guess is marked against the answer.
//!
//! Both lists are built into the program. The answers are common words, so a
//! round is always winnable; the guesses are a much longer list of real
//! words, so a player is rarely told that a word they know is not one.

use std::sync::LazyLock;

/// Letters in a word.
pub const LEN: usize = 5;
/// Guesses a player gets.
pub const TRIES: usize = 6;

const ANSWERS: &str = include_str!("answers.txt");
const GUESSES: &str = include_str!("guesses.txt");

/// A word, as five lower-case ASCII letters.
pub type Word = [u8; LEN];

fn parse(list: &'static str) -> Vec<Word> {
    list.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(word)
        .collect()
}

static ANSWER_LIST: LazyLock<Vec<Word>> = LazyLock::new(|| parse(ANSWERS));
/// Every word that may be guessed, answers included, sorted for searching.
static ACCEPTED: LazyLock<Vec<Word>> = LazyLock::new(|| {
    let mut all = parse(GUESSES);
    all.extend(ANSWER_LIST.iter());
    all.sort_unstable();
    all.dedup();
    all
});

/// Every word a round can be about.
#[must_use]
pub fn answers() -> &'static [Word] {
    &ANSWER_LIST
}

/// `text` as a word, if it is five letters. Case is ignored; whether it is a
/// real word is [`is_word`]'s question.
#[must_use]
pub fn word(text: &str) -> Option<Word> {
    let bytes: Word = text.as_bytes().try_into().ok()?;
    bytes
        .iter()
        .all(u8::is_ascii_alphabetic)
        .then(|| bytes.map(|b| b.to_ascii_lowercase()))
}

/// Whether `w` is in either list.
#[must_use]
pub fn is_word(w: &Word) -> bool {
    ACCEPTED.binary_search(w).is_ok()
}

/// The word, for showing: upper case, as the tiles are.
#[must_use]
pub fn shout(w: &Word) -> String {
    w.iter()
        .map(|b| char::from(b.to_ascii_uppercase()))
        .collect()
}

/// What a guess says about one of its letters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    /// Not in the answer, or not as many times as it is in the guess.
    Absent,
    /// In the answer, somewhere else.
    Present,
    /// In the answer, right here.
    Correct,
}

/// Marks each letter of `guess` against `answer`. A letter the answer has
/// once is marked once: the right place is marked first, then the leftmost
/// wrong places, and any more of it are absent.
#[must_use]
pub fn score(guess: &Word, answer: &Word) -> [Mark; LEN] {
    let mut marks = [Mark::Absent; LEN];
    // Letters of the answer not matched in place, left for the rest to find.
    let mut spare = [0u8; 26];
    for i in 0..LEN {
        if guess[i] == answer[i] {
            marks[i] = Mark::Correct;
        } else {
            spare[usize::from(answer[i] - b'a')] += 1;
        }
    }
    for i in 0..LEN {
        let left = &mut spare[usize::from(guess[i] - b'a')];
        if marks[i] != Mark::Correct && *left > 0 {
            *left -= 1;
            marks[i] = Mark::Present;
        }
    }
    marks
}

/// Whether `marks` say the guess was the answer.
#[must_use]
pub fn solved(marks: &[Mark; LEN]) -> bool {
    marks.iter().all(|&m| m == Mark::Correct)
}

/// In hard mode, every letter already found must be used: a green letter
/// kept in its place, and a yellow one somewhere. Why `word` falls short of
/// that after `earlier` guesses, if it does.
#[must_use]
pub fn hard_miss(earlier: &[(Word, [Mark; LEN])], word: &Word) -> Option<String> {
    for (guess, marks) in earlier {
        for i in 0..LEN {
            if marks[i] == Mark::Correct && word[i] != guess[i] {
                let c = char::from(guess[i].to_ascii_uppercase());
                return Some(format!("{} letter must be {c}", ordinal(i + 1)));
            }
        }
        for i in 0..LEN {
            if marks[i] == Mark::Absent {
                continue;
            }
            let letter = guess[i];
            let needed = (0..LEN)
                .filter(|&j| guess[j] == letter && marks[j] != Mark::Absent)
                .count();
            if word.iter().filter(|&&b| b == letter).count() < needed {
                let c = char::from(letter.to_ascii_uppercase());
                return Some(format!("guess must contain {c}"));
            }
        }
    }
    None
}

fn ordinal(n: usize) -> String {
    let suffix = match n {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}
