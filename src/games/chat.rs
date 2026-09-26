//! Talking across the table: what the two players have said to each other,
//! what is being typed, and the panel that shows both.
//!
//! Chat belongs to the table rather than to any game. A message goes over the
//! wire as `chat <text>`, and the table takes those lines before the game
//! sees anything, so `chat` is a word no game may use for itself. An older
//! build ignores the lines as it ignores anything it does not know, so
//! talking needs no new protocol version.

use std::cell::Cell;
use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::Ctx;
use crate::ui::{CURSOR, MUTED, cells};

/// The word a chat line starts with on the wire.
const WORD: &str = "chat";
/// The longest message either side keeps, in characters. Well inside
/// [`MAX_LINE`](crate::session::lines::MAX_LINE), even in four-byte UTF-8.
pub const MESSAGE_MAX: usize = 500;
/// How many messages are kept. Older ones scroll away for good.
const KEEP: usize = 200;
/// The most rows the message being typed grows to before it scrolls.
const COMPOSER_MAX: usize = 3;

/// The player's own messages sit in a bubble, the way a chat app draws them.
const BUBBLE: Color = Color::Rgb(58, 54, 50);
const BUBBLE_TEXT: Color = Color::Rgb(236, 232, 226);

/// One message, from either side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    /// Sent by this player, rather than the opponent.
    pub mine: bool,
    pub text: String,
}

/// What has been said, and what is being typed.
#[derive(Debug, Default)]
pub struct Chat {
    pub log: VecDeque<Said>,
    /// The message being typed.
    pub input: String,
    /// Keys go to the message being typed, rather than the game.
    pub focused: bool,
    /// Messages that arrived while the player was looking elsewhere.
    pub unread: usize,
    /// How many rows the log is scrolled back from its newest line.
    pub scroll: usize,
    /// How far back the log could scroll when it was last drawn. Drawing is
    /// the only place that knows how the messages wrap.
    max_scroll: Cell<usize>,
}

impl Chat {
    /// The text of a chat line from the peer, cleaned. `None` for a line
    /// that is not chat, or has nothing readable in it.
    #[must_use]
    pub fn parse(line: &str) -> Option<Option<String>> {
        let text = line.strip_prefix(WORD)?.strip_prefix(' ')?;
        Some(clean(text))
    }

    /// The line that carries `text` to the peer.
    #[must_use]
    pub fn line(text: &str) -> String {
        format!("{WORD} {text}")
    }

    /// A message from the opponent, already cleaned.
    pub fn heard(&mut self, text: String) {
        if !self.focused {
            self.unread += 1;
        }
        self.push(Said { mine: false, text });
    }

    /// A message of our own, which the peer has been sent.
    pub fn said(&mut self, text: String) {
        self.push(Said { mine: true, text });
    }

    fn push(&mut self, said: Said) {
        if self.log.len() == KEEP {
            self.log.pop_front();
        }
        self.log.push_back(said);
        // Someone reading back through the log keeps their place.
        if self.scroll > 0 {
            self.scroll += 1;
        }
    }

    pub fn focus(&mut self) {
        self.focused = true;
        self.unread = 0;
    }

    pub fn blur(&mut self) {
        self.focused = false;
    }

    /// Scrolls back through the log by `rows`, or forward if negative.
    pub fn scroll_by(&mut self, rows: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(rows)
            .min(self.max_scroll.get());
    }

    /// A key while typing. Returns a message to send once enter is pressed on
    /// one worth sending.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<String> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.blur(),
            KeyCode::Enter => {
                let text = clean(&self.input)?;
                self.input.clear();
                self.scroll = 0;
                return Some(text);
            }
            KeyCode::Backspace | KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char('u') if ctrl => self.input.clear(),
            KeyCode::Char(c) if !ctrl && !c.is_control() => {
                if self.input.chars().count() < MESSAGE_MAX {
                    self.input.push(c);
                }
            }
            KeyCode::Up => self.scroll_by(1),
            KeyCode::Down => self.scroll_by(-1),
            KeyCode::PageUp => self.scroll_by(5),
            KeyCode::PageDown => self.scroll_by(-5),
            _ => {}
        }
        None
    }

    fn delete_word(&mut self) {
        let kept = self.input.trim_end().len();
        self.input.truncate(kept);
        let start = self.input.rfind(' ').map_or(0, |i| i + 1);
        self.input.truncate(start);
    }
}

/// A message as it will be shown: one line, no control characters, runs of
/// whitespace squeezed to one space, and no longer than [`MESSAGE_MAX`].
/// `None` if nothing is left. What the peer sends is cleaned here, where it
/// arrives, rather than wherever it is drawn.
#[must_use]
pub fn clean(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MESSAGE_MAX)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// The chat panel: the conversation, newest at the bottom, and the message
/// being typed beneath it.
pub fn draw(f: &mut Frame, area: Rect, ctx: &Ctx) {
    let chat = &ctx.chat;
    let border = if chat.focused { CURSOR } else { MUTED };
    let mut title = vec![Span::raw(" chat ")];
    if chat.unread > 0 {
        title.push(Span::styled(
            format!("· {} new ", chat.unread),
            Style::default().fg(CURSOR).add_modifier(Modifier::BOLD),
        ));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Line::from(title));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 4 || inner.height < 3 {
        return;
    }
    let width = usize::from(inner.width);

    // The composer keeps its last rows, and leaves room for the rule and a
    // row of the log however short the panel is.
    let mut composer = composer_lines(ctx, width);
    let room = usize::from(inner.height - 2);
    if composer.len() > room {
        composer.drain(..composer.len() - room);
    }
    let composer_h = cells(composer.len());
    let log_h = inner.height - composer_h - 1;

    let log = log_lines(ctx, width);
    let visible = usize::from(log_h);
    let max_scroll = log.len().saturating_sub(visible);
    chat.max_scroll.set(max_scroll);
    let scroll = chat.scroll.min(max_scroll);
    let end = log.len() - scroll;
    let shown: Vec<Line> = log[end.saturating_sub(visible)..end].to_vec();
    // Messages settle at the bottom, next to where they are typed.
    let pad = log_h.saturating_sub(cells(shown.len()));
    f.render_widget(
        Paragraph::new(shown),
        Rect {
            y: inner.y + pad,
            height: log_h - pad,
            ..inner
        },
    );

    let rule_y = inner.y + log_h;
    let rule = if scroll > 0 {
        let more = format!(" ↓ {scroll} more ");
        let left = width.saturating_sub(more.chars().count()) / 2;
        let right = width.saturating_sub(left + more.chars().count());
        format!("{}{more}{}", "─".repeat(left), "─".repeat(right))
    } else {
        "─".repeat(width)
    };
    f.render_widget(
        Paragraph::new(Line::styled(rule, Style::default().fg(MUTED))),
        Rect {
            y: rule_y,
            height: 1,
            ..inner
        },
    );
    f.render_widget(
        Paragraph::new(composer),
        Rect {
            y: rule_y + 1,
            height: composer_h,
            ..inner
        },
    );
}

/// Every message, wrapped to `width`: the opponent's on the left under their
/// name, the player's own in a bubble on the right.
fn log_lines(ctx: &Ctx, width: usize) -> Vec<Line<'static>> {
    let chat = &ctx.chat;
    let muted = Style::default().fg(MUTED);
    if chat.log.is_empty() {
        let hint = if ctx.is_networked() {
            format!("say hello to {}", ctx.peer_label())
        } else {
            "chat opens once someone joins".into()
        };
        return wrap(&hint, width)
            .into_iter()
            .map(|l| Line::styled(l, muted).centered())
            .collect();
    }

    let name = Style::default().fg(CURSOR).add_modifier(Modifier::BOLD);
    let bubble = Style::default().bg(BUBBLE).fg(BUBBLE_TEXT);
    // A bubble takes most of the width, not all, so it reads as the
    // player's side of the conversation.
    let bubble_w = (width * 4 / 5).saturating_sub(2).max(1);
    let mut lines = Vec::new();
    let mut last: Option<bool> = None;
    for said in &chat.log {
        if last.is_some() {
            lines.push(Line::raw(""));
        }
        if said.mine {
            let rows = wrap(&said.text, bubble_w);
            let w = rows.iter().map(|r| text_width(r)).max().unwrap_or(0);
            for row in rows {
                let fill = " ".repeat(w - text_width(&row));
                lines.push(Line::styled(format!(" {row}{fill} "), bubble).right_aligned());
            }
        } else {
            if last != Some(false) {
                lines.push(Line::styled(ctx.peer_label(), name));
            }
            lines.extend(wrap(&said.text, width).into_iter().map(Line::raw));
        }
        last = Some(said.mine);
    }
    lines
}

/// The message being typed, after a prompt, with the cursor at its end, or a
/// hint at how to start one.
fn composer_lines(ctx: &Ctx, width: usize) -> Vec<Line<'static>> {
    let chat = &ctx.chat;
    let muted = Style::default().fg(MUTED);
    let prompt = Span::styled(
        "› ",
        Style::default().fg(if chat.focused { CURSOR } else { MUTED }),
    );
    let cursor = Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED));

    if chat.input.is_empty() {
        let hint = match (chat.focused, ctx.is_networked()) {
            (_, false) => "waiting for an opponent…".to_string(),
            (true, true) => format!("reply to {}", ctx.peer_label()),
            (false, true) => "t or click to chat".to_string(),
        };
        let mut spans = vec![prompt];
        if chat.focused {
            spans.push(cursor);
        }
        spans.push(Span::styled(hint, muted));
        return vec![Line::from(spans)];
    }

    // Two columns for the prompt, and one kept free for the cursor.
    let rows = wrap(&chat.input, width.saturating_sub(3).max(1));
    let skip = rows.len().saturating_sub(COMPOSER_MAX);
    let last = rows.len() - 1;
    rows.into_iter()
        .enumerate()
        .skip(skip)
        .map(|(i, row)| {
            let lead = if i == skip {
                prompt.clone()
            } else {
                Span::raw("  ")
            };
            let mut spans = vec![lead, Span::raw(row)];
            if i == last && chat.focused {
                spans.push(cursor.clone());
            }
            Line::from(spans)
        })
        .collect()
}

fn text_width(s: &str) -> usize {
    Line::raw(s).width()
}

/// `text` broken into rows no wider than `width`, between words where it
/// can be and inside a word too long for a row of its own. Spaces the
/// player is still typing are kept.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut row_w = 0;
    for (n, word) in text.split(' ').enumerate() {
        let gap = usize::from(n > 0 && !row.is_empty());
        let w = text_width(word);
        if row_w + gap + w <= width {
            if gap == 1 {
                row.push(' ');
            }
            row.push_str(word);
            row_w += gap + w;
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            row_w = 0;
        }
        for c in word.chars() {
            let cw = text_width(c.encode_utf8(&mut [0; 4]));
            if row_w + cw > width && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                row_w = 0;
            }
            row.push(c);
            row_w += cw;
        }
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_breaks_between_words_and_inside_long_ones() {
        assert_eq!(wrap("good luck have fun", 9), ["good luck", "have fun"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 5), [""]);
    }

    #[test]
    fn a_line_from_the_peer_is_chat_only_after_the_word_and_a_space() {
        assert_eq!(Chat::parse("chat hi"), Some(Some("hi".into())));
        assert_eq!(Chat::parse("chat \x1b[2Jhi"), Some(Some("[2Jhi".into())));
        assert_eq!(Chat::parse("chat   "), Some(None));
        assert_eq!(Chat::parse("chatter"), None);
        assert_eq!(Chat::parse("move e2e4"), None);
    }
}
