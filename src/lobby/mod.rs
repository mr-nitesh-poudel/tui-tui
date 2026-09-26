//! The first screen: a shelf of games along the top, and under it what can be
//! done with the one picked: host it, play it solo, or challenge a friend to
//! it. Beneath those, a box to type a code into and join, which needs no game
//! (the host names it); typing a code's number anywhere starts one there.
//!
//! Like a game, this only holds state and decides what a key or click means;
//! [`draw_lobby`] draws it and `main` acts on the [`Choice`] it returns.

use iroh::EndpointId;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::games::Kind;
use crate::profile::Contact;
use crate::session::Code;
use crate::session::code::{complete, is_prefix, is_word};

mod ui;

pub use ui::{LobbyGeometry, Shelf, draw_lobby};

/// Friends listed at once, most recently played first.
pub const FRIENDS_SHOWN: usize = 6;

use crate::session::name::NAME_MAX;

/// What can be done with the game picked, other than challenge a friend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Host,
    Local,
}

impl Item {
    /// In the order they are listed.
    pub const ALL: [Item; 2] = [Item::Host, Item::Local];

    /// What the item says. The status bar says what it means for the game
    /// picked: on your own, or two taking turns at one keyboard.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Item::Host => "Host a game",
            Item::Local => "Play solo",
        }
    }
}

/// One selectable thing in the lobby.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Item(Item),
    /// An index into [`Lobby::friends`].
    Friend(usize),
    /// The box a code is typed into, to join someone else's game.
    Code,
}

/// What is being typed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Code,
    Name,
}

/// What the player settled on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Host,
    Join(Code),
    Local,
    Quit,
    Challenge(EndpointId),
    Rename(String),
    Forget(EndpointId),
    AcceptInvite,
    DeclineInvite,
}

/// A friend waiting for an answer to their invite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invited {
    pub name: String,
    pub game: Kind,
}

/// How the code typed so far is looking, for the line under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Empty,
    Typing,
    Ready(Code),
    Bad(String),
}

pub struct Lobby {
    /// An index into [`Lobby::rows`].
    pub selected: usize,
    /// An index into [`Kind::ALL`].
    pub game: usize,
    pub editing: Option<Field>,
    /// The code typed so far.
    pub input: String,
    /// The name typed so far, while editing it.
    pub name_input: String,
    /// What we are called.
    pub name: String,
    /// Running without a saved profile, because another copy has it.
    pub guest: bool,
    /// Most recently played first.
    pub friends: Vec<Contact>,
    pub invite: Option<Invited>,
    /// A friend waiting on "forget them? y/n".
    pub forgetting: Option<usize>,
    /// Something to tell the player, until they press a key.
    pub notice: Option<String>,
    /// Last known terminal size, for turning clicks into rows.
    pub area: Rect,
}

impl Default for Lobby {
    fn default() -> Self {
        Self::new()
    }
}

impl Lobby {
    #[must_use]
    pub fn new() -> Self {
        Self {
            selected: 0,
            game: 0,
            editing: None,
            input: String::new(),
            name_input: String::new(),
            name: String::new(),
            guest: false,
            friends: Vec::new(),
            invite: None,
            forgetting: None,
            notice: None,
            area: Rect::new(0, 0, 80, 24),
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        let friends = self.friends.len().min(FRIENDS_SHOWN);
        Item::ALL
            .into_iter()
            .map(Row::Item)
            .chain((0..friends).map(Row::Friend))
            .chain([Row::Code])
            .collect()
    }

    /// The game hosting, a local game or a challenge would start.
    #[must_use]
    pub fn game(&self) -> Kind {
        Kind::ALL[self.game % Kind::ALL.len()]
    }

    /// The friend selected, if it is one.
    #[must_use]
    pub fn on_friend(&self) -> Option<usize> {
        match self.rows().get(self.selected) {
            Some(&Row::Friend(i)) => Some(i),
            _ => None,
        }
    }

    /// Steps through the games, wrapping round at either end.
    fn next_game(&mut self, step: isize) {
        let n = Kind::ALL.len().cast_signed();
        self.game = (self.game.cast_signed() + step).rem_euclid(n) as usize;
    }

    fn row_of(&self, row: Row) -> usize {
        self.rows().iter().position(|&r| r == row).unwrap_or(0)
    }

    /// Keep the selection on something that exists after the friends change.
    pub fn set_friends(&mut self, friends: Vec<Contact>) {
        let was = self.rows().get(self.selected).copied();
        self.friends = friends;
        let last = self.rows().len().saturating_sub(1);
        self.selected = match was {
            Some(row @ (Row::Item(_) | Row::Code)) => self.row_of(row),
            _ => self.selected.min(last),
        };
        self.forgetting = None;
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<Choice> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return Some(Choice::Quit);
        }
        self.notice = None;

        // An invite needs its answer before anything else.
        if self.invite.is_some() {
            return match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.invite = None;
                    Some(Choice::AcceptInvite)
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.invite = None;
                    Some(Choice::DeclineInvite)
                }
                _ => None,
            };
        }
        if let Some(i) = self.forgetting.take() {
            return match key.code {
                KeyCode::Char('y') => self.friends.get(i).map(|f| Choice::Forget(f.id)),
                _ => None,
            };
        }
        match self.editing {
            Some(Field::Code) => return self.code_key(key.code, ctrl),
            Some(Field::Name) => return self.name_key(key.code, ctrl),
            None => {}
        }

        let rows = self.rows();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = (self.selected + rows.len() - 1) % rows.len();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1) % rows.len();
            }
            // From what to do, to whom to challenge, to the code box.
            KeyCode::Tab => self.selected = self.next_section(1),
            KeyCode::BackTab => self.selected = self.next_section(-1),
            // The game can be changed from anywhere: everything below it
            // is about it.
            KeyCode::Left | KeyCode::Char('h') => self.next_game(-1),
            KeyCode::Right | KeyCode::Char('l') => self.next_game(1),
            KeyCode::Enter | KeyCode::Char(' ') => return self.activate(self.selected),
            KeyCode::Char('n') => self.rename(),
            KeyCode::Char('x') => self.forgetting = self.on_friend(),
            KeyCode::Char('q') | KeyCode::Esc => return Some(Choice::Quit),
            // Every code starts with its number, so typing one is as good as
            // choosing to join.
            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.edit(Field::Code);
                self.push(c);
            }
            _ => {}
        }
        None
    }

    fn code_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Choice> {
        match code {
            KeyCode::Esc => self.editing = None,
            KeyCode::Enter => {
                if let Entry::Ready(code) = self.entry() {
                    return Some(Choice::Join(code));
                }
            }
            KeyCode::Tab | KeyCode::Right => self.accept_completion(),
            KeyCode::Char('u') if ctrl => self.input.clear(),
            KeyCode::Char('w') if ctrl => {
                let trimmed = self.input.trim_end_matches('-');
                let keep = trimmed.rfind('-').map_or(0, |i| i + 1);
                self.input.truncate(keep);
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.push(c),
            _ => {}
        }
        None
    }

    fn name_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Choice> {
        match code {
            KeyCode::Esc => self.editing = None,
            KeyCode::Enter => {
                self.editing = None;
                return Some(Choice::Rename(self.name_input.clone()));
            }
            KeyCode::Char('u') if ctrl => self.name_input.clear(),
            KeyCode::Backspace => {
                self.name_input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => self.push_name(c),
            _ => {}
        }
        None
    }

    fn push_name(&mut self, c: char) {
        if self.name_input.chars().count() < NAME_MAX {
            self.name_input.push(c);
        }
    }

    pub fn on_paste(&mut self, text: &str) {
        if self.editing == Some(Field::Name) {
            for c in text.chars().filter(|c| !c.is_control()) {
                self.push_name(c);
            }
            return;
        }
        self.edit(Field::Code);
        self.input.clear();
        // People paste the whole command as often as the code.
        let text = text.rsplit_once("join").map_or(text, |(_, rest)| rest);
        for c in text.chars() {
            self.push(c);
        }
        self.input = self.input.trim_end_matches('-').to_string();
    }

    pub fn on_mouse(&mut self, ev: MouseEvent) -> Option<Choice> {
        if self.invite.is_some() {
            return None;
        }
        let g = LobbyGeometry::new(self.area, self);
        let at = Position::new(ev.column, ev.row);
        let hit = g.row_at(ev.column, ev.row);
        match ev.kind {
            MouseEventKind::Moved if self.editing.is_none() => {
                if let Some(i) = hit {
                    self.selected = i;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.notice = None;
                if let Some(i) = hit {
                    self.selected = i;
                    return self.activate(i);
                }
                if let Some(game) = g.card_at(ev.column, ev.row) {
                    self.game = game;
                } else if g.name.contains(at) {
                    self.rename();
                } else if g.input.contains(at) {
                    self.edit(Field::Code);
                }
            }
            _ => {}
        }
        None
    }

    /// The first row of the section `step` along from the selected one's,
    /// skipping the friends when there are none.
    fn next_section(&self, step: isize) -> usize {
        let rows = self.rows();
        let firsts: Vec<usize> = (0..rows.len())
            .filter(|&i| i == 0 || !same_section(rows[i - 1], rows[i]))
            .collect();
        let here = firsts
            .iter()
            .rposition(|&first| first <= self.selected)
            .unwrap_or(0);
        let n = firsts.len().cast_signed();
        firsts[(here.cast_signed() + step).rem_euclid(n) as usize]
    }

    fn activate(&mut self, i: usize) -> Option<Choice> {
        match *self.rows().get(i)? {
            Row::Item(Item::Host) => Some(Choice::Host),
            Row::Item(Item::Local) => Some(Choice::Local),
            Row::Friend(f) => self.friends.get(f).map(|f| Choice::Challenge(f.id)),
            Row::Code => {
                self.edit(Field::Code);
                None
            }
        }
    }

    fn rename(&mut self) {
        self.name_input = self.name.clone();
        self.edit(Field::Name);
    }

    fn edit(&mut self, field: Field) {
        self.editing = Some(field);
        if field == Field::Code {
            self.selected = self.row_of(Row::Code);
        }
    }

    /// Typed text goes in the way it will be read back: lower case, with any
    /// separator turned into a single dash.
    fn push(&mut self, c: char) {
        if c.is_ascii_alphanumeric() {
            self.input.push(c.to_ascii_lowercase());
        } else if (c == '-' || c == '_' || c == '.' || c.is_whitespace())
            && !self.input.is_empty()
            && !self.input.ends_with('-')
        {
            self.input.push('-');
        }
    }

    fn parts(&self) -> Vec<&str> {
        self.input.split('-').collect()
    }

    /// What the word being typed would become if Tab were pressed now.
    #[must_use]
    pub fn completion(&self) -> Option<&'static str> {
        let parts = self.parts();
        // The first part is the number, which has nothing to complete.
        let last = *parts.last()?;
        if parts.len() < 2 || last.is_empty() {
            return None;
        }
        complete(last).filter(|w| w.len() > last.len())
    }

    fn accept_completion(&mut self) {
        if let Some(word) = self.completion() {
            let typed = self.parts().last().map_or(0, |p| p.len());
            self.input.push_str(&word[typed..]);
        }
        // Move on to the next part, but only past one that is right.
        let parts = self.parts();
        let last = parts[parts.len() - 1];
        let valid = if parts.len() == 1 {
            last.parse::<u8>().is_ok_and(|n| n < 100)
        } else {
            is_word(last)
        };
        if valid && parts.len() < 4 {
            self.input.push('-');
        }
    }

    #[must_use]
    pub fn entry(&self) -> Entry {
        if self.input.is_empty() {
            return Entry::Empty;
        }
        if let Ok(code) = self.input.parse() {
            return Entry::Ready(code);
        }
        let parts = self.parts();
        if parts.len() > 4 {
            return Entry::Bad("a code is a number and three words".into());
        }
        // Only judge parts the player has moved on from.
        let done = parts.len() - 1;
        if done >= 1 && !parts[0].parse::<u8>().is_ok_and(|n| n < 100) {
            return Entry::Bad(format!("{:?} should be a number from 0 to 99", parts[0]));
        }
        if let Some(bad) = parts[1..done.max(1)].iter().find(|w| !is_word(w)) {
            return Entry::Bad(format!("{bad:?} is not one of the code words"));
        }
        // A last word that nothing starts with will never become one.
        let last = parts[done];
        if done >= 1 && !is_prefix(last) {
            return Entry::Bad(format!("no code word starts with {last:?}"));
        }
        Entry::Typing
    }
}

/// Whether two rows are in the same part of the screen.
fn same_section(a: Row, b: Row) -> bool {
    matches!(
        (a, b),
        (Row::Item(_), Row::Item(_)) | (Row::Friend(_), Row::Friend(_)) | (Row::Code, Row::Code)
    )
}
