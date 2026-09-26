//! Drawing a match of Wordle: the player's board in the middle, the
//! opponent's small beside it (or the score, alone), the keyboard underneath,
//! and a card when a round is settled.
//!
//! What is drawn and what can be clicked both come from [`Geometry`], so the
//! two cannot drift apart.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use super::app::{App, Face, Guess, Outcome, SPELL, Tone};
use super::font::Size;
use super::words::{LEN, Mark, TRIES};
use crate::games::{Conn, Ctx, chat, chrome};
use crate::ui::{BRIGHT, CAPTURE, CURSOR, MUTED, SELECTED, cells, centred, keycaps_fit};

const CORRECT: Color = Color::Rgb(83, 141, 78);
const PRESENT: Color = Color::Rgb(181, 159, 59);
const ABSENT: Color = Color::Rgb(58, 58, 60);
/// The outline of a tile with nothing in it.
const EDGE: Color = Color::Rgb(62, 62, 66);
/// The outline of a tile typed into.
const EDGE_TYPED: Color = Color::Rgb(140, 140, 146);
/// Tiles too small for an outline are filled instead.
const FILL_EMPTY: Color = Color::Rgb(36, 36, 40);
const FILL_TYPED: Color = Color::Rgb(74, 74, 80);
/// A key nothing is known about yet.
const KEY: Color = Color::Rgb(104, 104, 110);
const CARD_BG: Color = Color::Rgb(22, 22, 25);

/// Tile and key sizes, smallest first: the biggest that fits is used. A tile
/// one or two rows tall is a filled block; from three rows it is outlined,
/// and from six its letter is drawn big, in the 5x7 font. Anything smaller
/// gets the terminal's own letter, which is always the most legible.
const SCALES: [((u16, u16), (u16, u16)); 8] = [
    ((3, 1), (3, 1)),
    ((5, 2), (3, 1)),
    ((5, 3), (3, 1)),
    ((5, 3), (5, 2)),
    ((7, 3), (5, 2)),
    ((7, 3), (7, 3)),
    ((9, 6), (7, 3)),
    ((11, 6), (7, 3)),
];
/// Between tiles, and between keys.
const GAP: u16 = 1;
/// Between the board and whatever is beside it.
const SIDE_GAP: u16 = 4;
/// The opponent's board while the round is on: small, and colours only.
const MINI: (u16, u16) = (3, 1);
/// The score beside the board, alone: roomy, or at the least.
const STATS_W: u16 = 36;
const STATS_MIN_W: u16 = 24;
/// The border and a column of space either side of the small board, and of
/// the keyboard.
const PAD: u16 = 2;
/// Room for the share code while waiting for an opponent.
const WAIT_W: u16 = 27;
const ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const CHAT_MIN: u16 = 24;
const CHAT_MAX: u16 = 44;
const CHAT_MIN_H: u16 = 6;
/// Room kept either side of the game before the chat takes any more.
const MARGIN: u16 = 8;
const CARD_W: u16 = 52;

/// A key on the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Letter(u8),
    Enter,
    Back,
}

/// What sits beside the player's board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The score, alone.
    Stats,
    /// The share code, or how finding the opponent is going.
    Waiting,
    /// The opponent's board; `full` once the round is settled, when it grows
    /// to match ours if there is room.
    Theirs { full: bool },
}

/// Where everything sits this frame.
pub struct Geometry {
    pub title: Rect,
    /// Names over the boards, against someone.
    pub labels: bool,
    /// The player's own board, and the size of its tiles.
    pub mine: Rect,
    pub tile: (u16, u16),
    pub side: Rect,
    /// The size of the opponent's tiles, when the side is their board.
    pub side_tile: (u16, u16),
    /// Their board is small, in a box as tall as ours.
    pub side_boxed: bool,
    pub keys: Rect,
    /// The box around the keyboard, where there is room for it.
    pub keys_box: Rect,
    /// The size of a key.
    pub key: (u16, u16),
    pub status: Rect,
    /// Everything but the chat: the card is centred on it.
    pub play: Rect,
    /// Right of the game where there is room, else under it. `None` alone,
    /// and on a screen too small for it.
    pub chat: Option<Rect>,
    pub footer: Rect,
}

impl Geometry {
    /// The layout for this match at this table.
    pub fn of(area: Rect, ctx: &Ctx, app: &App) -> Self {
        let side = if app.alone() {
            Side::Stats
        } else if ctx.peer.is_none() {
            Side::Waiting
        } else {
            Side::Theirs {
                full: app.round.as_ref().is_some_and(super::app::Round::over),
            }
        };
        Self::layout(area, ctx.has_chat() && !app.alone(), side)
    }

    /// Picks the biggest tiles and keys that fit, keeping room for the chat
    /// beside them if it can. The board and `side`, as tall as each other,
    /// go in the middle together, with the keyboard boxed underneath.
    #[must_use]
    pub fn layout(area: Rect, chat: bool, side: Side) -> Self {
        let [main, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        let labels = side != Side::Stats;
        let side_min = match side {
            Side::Stats => STATS_MIN_W,
            Side::Waiting => WAIT_W,
            Side::Theirs { .. } => board_w(MINI.0) + 2 * PAD,
        };
        let content_w = |(tile, key): ((u16, u16), (u16, u16))| {
            (board_w(tile.0) + SIDE_GAP + side_min).max(keys_w(key.0) + 2 * PAD)
        };
        let fits = |extra: u16| {
            SCALES.iter().rposition(|&s| {
                content_w(s) + extra <= main.width && column_h(s.0.1, s.1.1, labels) <= main.height
            })
        };
        // The chat goes beside the game if that leaves the tiles room to be
        // outlined, or costs them nothing; otherwise the tiles come first.
        let alone = fits(0).unwrap_or(0);
        let (scale, beside) = match fits(CHAT_MIN + 2) {
            Some(i) if chat && (i == alone || SCALES[i].0.1 >= 3) => (i, true),
            _ => (alone, false),
        };
        let (tile, key) = SCALES[scale];
        let (bw, bh) = (board_w(tile.0), board_h(tile.1));
        let kw = keys_w(key.0);
        let base_w = content_w(SCALES[scale]).min(main.width);
        let col_h = column_h(tile.1, key.1, labels).min(main.height);

        // The chat grows to its widest only once the rest has a margin.
        let chat_w = if beside {
            (main.width - base_w - 2)
                .saturating_sub(MARGIN)
                .clamp(CHAT_MIN, CHAT_MAX)
        } else {
            0
        };
        let spare_h = main.height - col_h;
        let below = chat && !beside && spare_h >= CHAT_MIN_H;
        let chat_h = if below { spare_h } else { 0 };
        let play = Rect {
            width: main.width - if beside { chat_w + 2 } else { 0 },
            height: main.height - chat_h,
            ..main
        };
        let top = play.y + (play.height - col_h) / 2;

        // Once the round is settled their board grows, as big as ours if
        // there is room, else as big as there is room for.
        let grown = (side == Side::Theirs { full: true })
            .then(|| {
                let room = play.width.saturating_sub(bw + SIDE_GAP);
                SCALES
                    .iter()
                    .rev()
                    .map(|&(t, _)| t)
                    .find(|&t| t.0 <= tile.0 && t.1 <= tile.1 && board_w(t.0) <= room)
            })
            .flatten()
            .filter(|&t| t != MINI);
        let full = grown.is_some();
        let side_w = match side {
            Side::Stats if play.width >= bw + SIDE_GAP + STATS_W => STATS_W,
            Side::Stats => STATS_MIN_W,
            Side::Waiting => WAIT_W,
            Side::Theirs { .. } if full => board_w(grown.unwrap_or(MINI).0),
            Side::Theirs { .. } => board_w(MINI.0) + 2 * PAD,
        };
        // Small, their tiles are as tall as the box leaves room for, with the
        // line under them.
        let side_tile = grown.unwrap_or(if bh >= board_h(2) + 4 {
            (MINI.0, 2)
        } else {
            MINI
        });
        let side_boxed =
            matches!(side, Side::Theirs { .. }) && !full && bh >= board_h(side_tile.1) + 2;

        // The board and what is beside it are centred together, over the
        // keyboard.
        let pair_w = bw + SIDE_GAP + side_w;
        let board_x = play.x + play.width.saturating_sub(pair_w) / 2;
        let board_y = top + 2 + u16::from(labels);
        let mine = Rect::new(board_x, board_y, bw, bh).intersection(main);
        // Whatever is beside the board is as tall as it, so the two line up
        // top and bottom; only their board grown smaller than ours is not.
        let side_h = if full { board_h(side_tile.1) } else { bh };
        let side_rect =
            Rect::new(board_x + bw + SIDE_GAP, board_y, side_w, side_h).intersection(main);

        let keys_y = board_y + bh + 1;
        let keys_box = Rect::new(
            play.x + play.width.saturating_sub(kw + 2 * PAD) / 2,
            keys_y,
            kw + 2 * PAD,
            3 * key.1 + 2,
        )
        .intersection(main);
        let keys = Rect::new(keys_box.x + PAD, keys_y + 1, kw, 3 * key.1).intersection(main);
        let status =
            Rect::new(play.x, keys_y + 3 * key.1 + 2 + 1, play.width, 1).intersection(main);
        let heading = Rect::new(play.x, top, play.width, 1).intersection(main);

        let chat = if beside {
            Some(Rect::new(main.right() - chat_w - 1, top, chat_w, col_h))
        } else if below {
            Some(Rect::new(
                main.x + (main.width - base_w) / 2,
                main.y + col_h,
                base_w,
                chat_h,
            ))
        } else {
            None
        }
        .map(|r| r.intersection(main));

        Self {
            title: heading,
            labels,
            mine,
            tile,
            side: side_rect,
            side_tile,
            side_boxed,
            keys,
            keys_box,
            key,
            status,
            play,
            chat,
            footer,
        }
    }

    /// Every key, and where it is.
    #[must_use]
    pub fn key_rects(&self) -> Vec<(Key, Rect)> {
        let (kw, kh) = self.key;
        let wide = wide_key(kw);
        let mut out = Vec::new();
        for (r, letters) in ROWS.iter().enumerate() {
            let y = self.keys.y + cells(r) * kh;
            let mut x = self.keys.x;
            match r {
                // Tucked in by half a key, as a keyboard is.
                1 => x += u16::midpoint(kw, GAP),
                2 => {
                    out.push((Key::Enter, Rect::new(x, y, wide, kh)));
                    x += wide + GAP;
                }
                _ => {}
            }
            for b in letters.bytes() {
                out.push((Key::Letter(b), Rect::new(x, y, kw, kh)));
                x += kw + GAP;
            }
            if r == 2 {
                out.push((Key::Back, Rect::new(x, y, wide, kh)));
            }
        }
        out.into_iter()
            .map(|(k, r)| (k, r.intersection(self.keys)))
            .filter(|(_, r)| !r.is_empty())
            .collect()
    }

    /// The key under a screen position, if any.
    #[must_use]
    pub fn key_at(&self, x: u16, y: u16) -> Option<Key> {
        self.key_rects()
            .into_iter()
            .find(|(_, r)| r.contains((x, y).into()))
            .map(|(k, _)| k)
    }
}

fn board_w(tile_w: u16) -> u16 {
    cells(LEN) * tile_w + cells(LEN - 1) * GAP
}

fn board_h(tile_h: u16) -> u16 {
    cells(TRIES) * tile_h
}

fn keys_w(key_w: u16) -> u16 {
    10 * key_w + 9 * GAP
}

/// Enter and backspace, either end of the bottom row, fill it out to the
/// width of the top one.
fn wide_key(key_w: u16) -> u16 {
    (3 * key_w).div_ceil(2)
}

/// The title and a gap, the names, the board, a gap, the keyboard in its
/// box, a gap and the status line.
fn column_h(tile_h: u16, key_h: u16, labels: bool) -> u16 {
    2 + u16::from(labels) + board_h(tile_h) + 1 + 3 * key_h + 2 + 1 + 1
}

fn tile_rect(board: Rect, (w, h): (u16, u16), row: usize, col: usize) -> Rect {
    Rect::new(
        board.x + cells(col) * (w + GAP),
        board.y + cells(row) * h,
        w,
        h,
    )
}

pub fn draw(f: &mut Frame, app: &App, ctx: &Ctx) {
    let g = Geometry::of(f.area(), ctx, app);
    let now = app.now();

    f.render_widget(Paragraph::new(title(app, ctx)).centered(), g.title);
    if g.labels {
        draw_labels(f, &g, ctx);
    }
    draw_mine(f.buffer_mut(), &g, app, now);
    if app.alone() {
        draw_stats(f, g.side, app);
    } else if ctx.peer.is_some() {
        draw_theirs(f.buffer_mut(), &g, app, now);
    } else {
        draw_waiting(f, g.side, ctx);
    }
    f.render_widget(panel(), g.keys_box);
    draw_keys(f.buffer_mut(), &g, app, now);

    let (text, tone) = app.status(ctx);
    f.render_widget(
        Paragraph::new(Line::styled(text, tone_style(tone))).centered(),
        g.status,
    );
    if let Some(area) = g.chat {
        chat::draw(f, area, ctx);
    }
    if let Some(since) = app.card_since() {
        draw_card(f, g.play, app, ctx, since);
    }
    draw_footer(f, g.footer, app, ctx);
}

fn title(app: &App, ctx: &Ctx) -> Line<'static> {
    let muted = Style::default().fg(MUTED);
    let bold = Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled("WORDLE", bold)];
    let hard = app
        .round
        .as_ref()
        .filter(|_| app.in_play())
        .map_or(app.hard, |r| r.hard);
    if let Some(r) = &app.round {
        spans.push(Span::styled(format!("  ·  round {}", r.number), muted));
    }
    if hard {
        spans.push(Span::styled("  ·  ", muted));
        spans.push(Span::styled("hard mode", Style::default().fg(CURSOR)));
    }
    if !app.alone() && app.score.played() > 0 {
        let s = &app.score;
        spans.push(Span::styled("  ·  ", muted));
        spans.push(Span::styled(format!("you {}", s.won), bold));
        spans.push(Span::styled(" – ", muted));
        spans.push(Span::styled(
            format!("{} {}", s.lost, ctx.peer_label()),
            bold,
        ));
        if s.drawn > 0 {
            spans.push(Span::styled(format!("  ({} drawn)", s.drawn), muted));
        }
    }
    Line::from(spans)
}

/// Names over the two boards.
fn draw_labels(f: &mut Frame, g: &Geometry, ctx: &Ctx) {
    let muted = Style::default().fg(MUTED);
    let above = |r: Rect| Rect {
        y: r.y.saturating_sub(1),
        height: 1,
        ..r
    };
    f.render_widget(
        Paragraph::new(Line::styled("you", muted)).centered(),
        above(g.mine),
    );
    if ctx.peer.is_none() {
        return;
    }
    let mut theirs = Vec::new();
    if let Some(dot) = chrome::connection_dot(ctx) {
        theirs.push(dot);
        theirs.push(Span::raw(" "));
    }
    theirs.push(Span::styled(ctx.peer_label(), muted));
    // The name may be wider than a small board: it spills to the right.
    let at = Rect {
        width: g.side.width.max(cells(Line::from(theirs.clone()).width())),
        ..above(g.side)
    }
    .intersection(f.area());
    f.render_widget(Paragraph::new(Line::from(theirs)).centered(), at);
}

/// The faint rounded box every panel beside or under the board sits in.
fn panel() -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(EDGE))
}

/// Fills `r` with `colour`, as a solid block trimmed in half blocks top and
/// bottom once it is tall enough, which leaves half a row between one block
/// and the next. Tiles and keys are both drawn this way.
fn fill_block(buf: &mut Buffer, r: Rect, colour: Color) {
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            let cell = &mut buf[(x, y)];
            if r.height >= 2 && y == r.top() {
                cell.set_char('▄').set_style(Style::default().fg(colour));
            } else if r.height >= 3 && y + 1 == r.bottom() {
                cell.set_char('▀').set_style(Style::default().fg(colour));
            } else {
                cell.set_char(' ').set_style(Style::default().bg(colour));
            }
        }
    }
}

fn mark_colour(m: Mark) -> Color {
    match m {
        Mark::Correct => CORRECT,
        Mark::Present => PRESENT,
        Mark::Absent => ABSENT,
    }
}

/// How a tile looks, before any turning over.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    Empty,
    /// Typed into; `pop` for the letter just typed.
    Typed {
        pop: bool,
    },
    /// Turned over to show what the guess found.
    Marked(Color),
}

/// One tile, squashed to `squash` of its height as it turns over: a filled
/// block when it is one row tall, else an outline until it is marked and a
/// solid block after. The solid block is drawn in half blocks top and bottom,
/// leaving the same half row between tiles that an outline does.
fn draw_tile(buf: &mut Buffer, r: Rect, letter: Option<u8>, look: Look, squash: f32) {
    let r = r.intersection(buf.area);
    if r.is_empty() {
        return;
    }
    let bold = Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD);
    // Two rows is too few for an outline, so the tile is filled instead.
    let look = match (r.height, look) {
        (2, Look::Empty) => Look::Marked(FILL_EMPTY),
        (2, Look::Typed { pop }) => Look::Marked(if pop { EDGE_TYPED } else { FILL_TYPED }),
        _ => look,
    };
    if r.height == 1 {
        let bg = match look {
            Look::Empty => FILL_EMPTY,
            Look::Typed { .. } => FILL_TYPED,
            Look::Marked(c) => c,
        };
        fill_block(buf, r, bg);
        // Mid-turn, the letter is edge on.
        if let Some(b) = letter.filter(|_| squash > 0.4) {
            buf[(r.x + r.width / 2, r.y)]
                .set_char(char::from(b.to_ascii_uppercase()))
                .set_style(bold);
        }
        return;
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a fraction of a tile's height, which is a few rows"
    )]
    let keep = ((f32::from(r.height) * squash).round() as u16).clamp(1, r.height);
    let r = Rect {
        y: r.y + (r.height - keep) / 2,
        height: keep,
        ..r
    };
    let edge = match look {
        Look::Empty => EDGE,
        Look::Typed { pop: true } => BRIGHT,
        Look::Typed { pop: false } => EDGE_TYPED,
        Look::Marked(c) => c,
    };
    if r.height == 1 {
        for x in r.left()..r.right() {
            buf[(x, r.y)]
                .set_char('━')
                .set_style(Style::default().fg(edge));
        }
        return;
    }
    let text_style = if let Look::Marked(c) = look {
        fill_block(buf, r, c);
        bold.bg(c)
    } else {
        let kind = if look == (Look::Typed { pop: true }) {
            BorderType::Thick
        } else {
            BorderType::Rounded
        };
        Block::bordered()
            .border_type(kind)
            .border_style(Style::default().fg(edge))
            .render(r, buf);
        bold
    };
    let Some(b) = letter else { return };
    let c = char::from(b.to_ascii_uppercase());
    let big = Size::Large;
    if r.height >= big.height() + 2 && r.width >= big.width("W") + 2 {
        let glyph = c.to_string();
        big.draw(
            buf,
            r.x + (r.width - big.width(&glyph)) / 2,
            r.y + (r.height - big.height()) / 2,
            &glyph,
            text_style,
        );
    } else {
        buf[(r.x + r.width / 2, r.y + r.height / 2)]
            .set_char(c)
            .set_style(text_style);
    }
}

/// A guess's tile `col`, as it looks at `now`: its letter if `letters`, and
/// how far it has turned over.
fn guess_tile(guess: &Guess, col: usize, now: Instant, letters: bool) -> (Option<u8>, Look, f32) {
    let letter = letters.then_some(guess.word[col]);
    let marked = Look::Marked(mark_colour(guess.marks[col]));
    match guess.face(col, now) {
        Face::Up => (letter, Look::Typed { pop: false }, 1.0),
        // Face down it squashes to a line, then opens out in its colour.
        Face::Turning(t) if t < 0.5 => (letter, Look::Typed { pop: false }, 1.0 - 2.0 * t),
        Face::Turning(t) => (letter, marked, 2.0 * t - 1.0),
        Face::Down => (letter, marked, 1.0),
    }
}

fn draw_mine(buf: &mut Buffer, g: &Geometry, app: &App, now: Instant) {
    let guesses = app.round.as_ref().map_or(&[][..], |r| &r.me.guesses[..]);
    let bounce = app.bounce();
    let draw_row = |buf: &mut Buffer, row: usize, lifted: [bool; LEN]| {
        let typing = row == guesses.len() && app.guessing();
        let shift = if typing { app.shake_offset() } else { 0 };
        for (col, up) in lifted.into_iter().enumerate() {
            let mut r = tile_rect(g.mine, g.tile, row, col);
            r.x = r.x.saturating_add_signed(shift);
            // Only an outlined tile hops: a one-row tile would land on the row
            // above.
            if up && g.tile.1 >= 3 {
                r.y = r.y.saturating_sub(1);
            }
            let (letter, look, squash) = if let Some(guess) = guesses.get(row) {
                guess_tile(guess, col, now, true)
            } else if typing {
                match app.typing.get(col) {
                    Some(&b) => (
                        Some(b),
                        Look::Typed {
                            pop: app.popping(col),
                        },
                        1.0,
                    ),
                    None => (None, Look::Empty, 1.0),
                }
            } else {
                (None, Look::Empty, 1.0)
            };
            draw_tile(buf, r, letter, look, squash);
        }
    };
    for row in 0..TRIES {
        if bounce.is_none_or(|(hopping, _)| hopping != row) {
            draw_row(buf, row, [false; LEN]);
        }
    }
    // The hopping row last, so it lands on top of the one above.
    if let Some((row, lifted)) = bounce {
        draw_row(buf, row, lifted);
    }
}

/// The opponent's board: colours only, until ours is done and the letters
/// can give nothing away. While it is small it sits in a box as tall as ours,
/// with a line under it on how they are doing.
fn draw_theirs(buf: &mut Buffer, g: &Geometry, app: &App, now: Instant) {
    let round = app.round.as_ref();
    let guesses = round
        .and_then(|r| r.them.as_ref())
        .map_or(&[][..], |b| &b.guesses[..]);
    let letters = round.is_some_and(super::app::Round::show_theirs);
    let (w, h) = (board_w(g.side_tile.0), board_h(g.side_tile.1));
    let mut at = g.side;
    if g.side_boxed {
        panel().render(g.side, buf);
        let inner = g.side.inner(ratatui::layout::Margin::new(1, 1));
        // The grid, a blank row and the line, in the middle of the box.
        let with_line = inner.height >= h + 2;
        let content = if with_line { h + 2 } else { h };
        let top = inner.y + (inner.height - content) / 2;
        at = Rect::new(inner.x + inner.width.saturating_sub(w) / 2, top, w, h);
        if with_line {
            let (text, style) = their_progress(app);
            Paragraph::new(Line::styled(text, style))
                .centered()
                .render(Rect::new(inner.x, top + h + 1, inner.width, 1), buf);
        }
    }
    for row in 0..TRIES {
        for col in 0..LEN {
            let r = tile_rect(at, g.side_tile, row, col);
            let (letter, look, squash) = match guesses.get(row) {
                Some(guess) => guess_tile(guess, col, now, letters),
                None => (None, Look::Empty, 1.0),
            };
            draw_tile(buf, r, letter, look, squash);
        }
    }
}

/// How the opponent is doing, in a few words.
fn their_progress(app: &App) -> (String, Style) {
    let muted = Style::default().fg(MUTED);
    let ready = (
        "ready to play".to_string(),
        Style::default().fg(CURSOR).add_modifier(Modifier::BOLD),
    );
    let Some(round) = &app.round else {
        return if app.setup.theirs.is_some() {
            ready
        } else {
            ("not ready yet".into(), muted)
        };
    };
    if round.over() && app.setup.theirs.is_some() {
        return ready;
    }
    let Some(them) = &round.them else {
        return (String::new(), muted);
    };
    match them.solved_in() {
        Some((1, _)) => ("found it first go".into(), Style::default().fg(SELECTED)),
        Some((n, _)) => (format!("found it in {n}"), Style::default().fg(SELECTED)),
        None if them.done() => ("out of guesses".into(), Style::default().fg(CAPTURE)),
        None => (format!("{} of {TRIES} guesses", them.guesses.len()), muted),
    }
}

/// Before the opponent arrives: the code to send them, or how finding them
/// is going.
fn draw_waiting(f: &mut Frame, area: Rect, ctx: &Ctx) {
    let block = panel().title(Line::styled(" opponent ", Style::default().fg(MUTED)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines = chrome::connection_lines(ctx);
    let h = cells(lines.len()).min(inner.height);
    let at = Rect {
        y: inner.y + (inner.height - h) / 2,
        height: h,
        ..inner
    };
    f.render_widget(Paragraph::new(lines).centered(), at);
}

/// Alone, the score takes the opponent's place, in a card as tall as the
/// board: four numbers, big where there is room, over how many guesses each
/// word took. What is spare of the height goes evenly above, between and
/// below the two.
fn draw_stats(f: &mut Frame, area: Rect, app: &App) {
    let chart_h = 1 + cells(TRIES);
    let inner = if area.height >= 2 + 2 + chart_h && area.width >= 22 {
        let block =
            panel().title(Line::styled(" your words ", Style::default().fg(MUTED)).centered());
        let inner = block.inner(area);
        f.render_widget(block, area);
        inner
    } else {
        area
    };
    let big = inner.width >= 30 && inner.height >= 9 + chart_h;
    let stats_h = if big { 9 } else { 2 };
    let spare = inner.height.saturating_sub(stats_h + chart_h);
    let stats_y = inner.y + spare / 3;
    let chart_y = stats_y + stats_h + spare / 3;

    // Two columns of two. Big, each number is centred over what it is;
    // small, the numbers line up on the right and what they are on the left,
    // and each column is centred as a whole.
    let buf = f.buffer_mut();
    let col_w = inner.width / 2;
    let values = stat_values(app);
    for (i, (value, what)) in values.iter().enumerate() {
        let x = inner.x + cells(i % 2) * col_w;
        if big {
            let y = stats_y + cells(i / 2) * 5;
            big_stat(buf, Rect::new(x, y, col_w, 4), value, what);
            continue;
        }
        let column = values.iter().skip(i % 2).step_by(2);
        let value_w = column.clone().map(|(v, _)| v.len()).max().unwrap_or(0);
        let what_w = column.map(|(_, w)| w.len()).max().unwrap_or(0);
        let left = col_w.saturating_sub(cells(value_w + 1 + what_w)) / 2;
        let line = Line::from(vec![
            Span::styled(
                format!("{value:>value_w$}"),
                Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {what}"), Style::default().fg(MUTED)),
        ]);
        let at = Rect::new(x + left, stats_y + cells(i / 2), col_w - left, 1).intersection(inner);
        Paragraph::new(line).render(at, buf);
    }

    // A rule with its heading, then the chart, inset from the edges.
    let inset =
        Rect::new(inner.x + 2, chart_y, inner.width.saturating_sub(4), chart_h).intersection(inner);
    if inset.is_empty() {
        return;
    }
    rule(buf, Rect { height: 1, ..inset }, "guesses");
    let chart = Rect {
        y: inset.y + 1,
        height: inset.height - 1,
        ..inset
    };
    draw_chart(buf, chart, &app.score.spread, latest_solve(app));
}

/// A faint rule across `r` with `label` in the middle of it.
fn rule(buf: &mut Buffer, r: Rect, label: &str) {
    let label = format!(" {label} ");
    let w = usize::from(r.width);
    let lw = label.chars().count().min(w);
    let left = (w - lw) / 2;
    let line = Line::from(vec![
        Span::styled("─".repeat(left), Style::default().fg(EDGE)),
        Span::styled(label, Style::default().fg(MUTED)),
        Span::styled("─".repeat(w - lw - left), Style::default().fg(EDGE)),
    ]);
    Paragraph::new(line).render(r, buf);
}

/// The four numbers on the score card, and what each is.
fn stat_values(app: &App) -> [(String, &'static str); 4] {
    let s = &app.score;
    let rate = (s.won * 100).checked_div(s.played()).unwrap_or(0);
    [
        (s.played().to_string(), "played"),
        (format!("{rate}%"), "found"),
        (s.streak.to_string(), "in a row"),
        (s.best_streak.to_string(), "best run"),
    ]
}

/// A number in big digits over what it is, both centred in `r`.
fn big_stat(buf: &mut Buffer, r: Rect, value: &str, what: &str) {
    let w = Size::Small.width(value);
    Size::Small.draw(
        buf,
        r.x + r.width.saturating_sub(w) / 2,
        r.y,
        value,
        Style::default().fg(BRIGHT),
    );
    let y = r.y + Size::Small.height();
    let at = Rect::new(r.x, y, r.width, 1).intersection(buf.area);
    Paragraph::new(Line::styled(what, Style::default().fg(MUTED)))
        .centered()
        .render(at, buf);
}

/// How many guesses the word the last round was about took, if it was found.
fn latest_solve(app: &App) -> Option<usize> {
    app.round
        .as_ref()
        .filter(|r| r.over())
        .and_then(|r| r.me.solved_in())
        .map(|(n, _)| n)
}

/// A bar for each number of guesses, as long as how many words took that
/// many, with the count at its end; the latest word's bar in green. Every
/// bar shows, however short, so a zero still reads as a row.
fn draw_chart(buf: &mut Buffer, area: Rect, spread: &[u32; TRIES], latest: Option<usize>) {
    let area = area.intersection(buf.area);
    let most = spread.iter().copied().max().unwrap_or(0).max(1);
    let count_w = cells(most.to_string().len());
    // The number of guesses and a space, then the bar, then room for a
    // count after it.
    let room = area.width.saturating_sub(2 + count_w + 1);
    if room == 0 {
        return;
    }
    let muted = Style::default().fg(MUTED);
    for (i, &n) in spread.iter().enumerate() {
        let y = area.y + cells(i);
        if y >= area.bottom() {
            break;
        }
        let colour = if latest == Some(i + 1) {
            CORRECT
        } else {
            ABSENT
        };
        buf.set_string(area.x, y, (i + 1).to_string(), muted);
        let len = u16::try_from(u32::from(room) * n / most)
            .unwrap_or(room)
            .clamp(1, room);
        let bar = Rect::new(area.x + 2, y, len, 1);
        for x in bar.left()..bar.right() {
            buf[(x, y)]
                .set_char(' ')
                .set_style(Style::default().bg(colour));
        }
        let count = n.to_string();
        let count_style = if n > 0 {
            Style::default().fg(BRIGHT)
        } else {
            muted
        };
        buf.set_string(bar.right() + 1, y, &count, count_style);
    }
}

fn draw_keys(buf: &mut Buffer, g: &Geometry, app: &App, now: Instant) {
    let board = app.round.as_ref().map(|r| &r.me);
    for (key, r) in g.key_rects() {
        let (label, bg, fg) = match key {
            Key::Letter(b) => {
                let known = board.and_then(|board| board.letter(b, now));
                let (bg, fg) = match known {
                    None => (KEY, BRIGHT),
                    Some(Mark::Absent) => (ABSENT, MUTED),
                    Some(m) => (mark_colour(m), BRIGHT),
                };
                (char::from(b.to_ascii_uppercase()).to_string(), bg, fg)
            }
            Key::Enter if r.width >= 5 => ("enter".into(), KEY, BRIGHT),
            Key::Enter => ("↵".into(), KEY, BRIGHT),
            Key::Back => ("⌫".into(), KEY, BRIGHT),
        };
        draw_key(buf, r, &label, bg, fg);
    }
}

/// A key: a block of `bg` with its label on, trimmed in half blocks when it
/// is tall enough to leave a gap between rows of keys.
fn draw_key(buf: &mut Buffer, r: Rect, label: &str, bg: Color, fg: Color) {
    let r = r.intersection(buf.area);
    if r.is_empty() {
        return;
    }
    fill_block(buf, r, bg);
    let w = cells(label.chars().count());
    let style = Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD);
    buf.set_stringn(
        r.x + r.width.saturating_sub(w) / 2,
        r.y + r.height / 2,
        label,
        usize::from(r.width),
        style,
    );
}

/// The card over the boards once a round is settled: the verdict, the word
/// spelt out, the score, and what to do next.
fn draw_card(f: &mut Frame, play: Rect, app: &App, ctx: &Ctx, since: Duration) {
    let Some(round) = &app.round else { return };
    let peer = ctx.peer_label();
    let (headline, colour) = match (app.alone(), round.outcome()) {
        (true, Some(Outcome::Won)) => ("GOT IT", SELECTED),
        (true, _) => ("MISSED", CAPTURE),
        (false, Some(Outcome::Won)) => ("YOU WIN", SELECTED),
        (false, Some(Outcome::Lost)) => ("YOU LOSE", CAPTURE),
        (false, _) => ("DRAW", CURSOR),
    };
    let (detail, _) = App::verdict(round, &peer);
    let big = play.width >= CARD_W && play.height >= 28;
    let chart = app.alone() && big;
    let word_tile = if big { (5, 3) } else { (3, 1) };

    let headline_h = if big { Size::Large.height() } else { 1 };
    let score_h = if chart { 2 + cells(TRIES) } else { 1 };
    // Borders, and a blank row between each part.
    let h = 2 + 1 + headline_h + 1 + 1 + 1 + word_tile.1 + 1 + score_h + 1 + 1 + 1 + 1;
    let area = centred(play, CARD_W, h);
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colour))
        .style(Style::default().bg(CARD_BG));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let buf = f.buffer_mut();
    let line = |buf: &mut Buffer, y: u16, line: Line| {
        if y < inner.bottom() {
            Paragraph::new(line)
                .centered()
                .render(Rect::new(inner.x, y, inner.width, 1), buf);
        }
    };
    let mut y = inner.y + 1;

    if big {
        let x = inner.x + inner.width.saturating_sub(Size::Large.width(headline)) / 2;
        Size::Large.draw(buf, x, y, headline, Style::default().fg(colour));
    } else {
        let bold = Style::default().fg(colour).add_modifier(Modifier::BOLD);
        line(buf, y, Line::styled(headline, bold));
    }
    y += headline_h + 1;
    line(buf, y, Line::styled(detail, Style::default().fg(BRIGHT)));
    y += 2;

    // The word, a letter at a time.
    let word_w = board_w(word_tile.0);
    let word = Rect::new(
        inner.x + inner.width.saturating_sub(word_w) / 2,
        y,
        word_w,
        word_tile.1,
    );
    let spelt = usize::try_from(since.as_millis() / SPELL.as_millis().max(1)).unwrap_or(LEN);
    for col in 0..LEN {
        let r = tile_rect(word, word_tile, 0, col).intersection(inner);
        if col < spelt {
            let letter = Some(round.answer[col]);
            draw_tile(buf, r, letter, Look::Marked(CORRECT), 1.0);
        } else {
            draw_tile(buf, r, None, Look::Empty, 1.0);
        }
    }
    y += word_tile.1 + 1;

    let muted = Style::default().fg(MUTED);
    let bright = Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD);
    let s = &app.score;
    if app.alone() {
        let [played, found, run, best] = stat_values(app);
        let spans = vec![
            Span::styled(played.0, bright),
            Span::styled(" played · ", muted),
            Span::styled(found.0, bright),
            Span::styled(" found · run ", muted),
            Span::styled(run.0, bright),
            Span::styled(" · best ", muted),
            Span::styled(best.0, bright),
        ];
        line(buf, y, Line::from(spans));
        if chart {
            let at = Rect::new(
                inner.x + 4,
                y + 2,
                inner.width.saturating_sub(8),
                cells(TRIES),
            )
            .intersection(inner);
            draw_chart(buf, at, &s.spread, latest_solve(app));
        }
    } else {
        let mut spans = vec![
            Span::styled("you ", muted),
            Span::styled(s.won.to_string(), bright),
            Span::styled("  ·  ", muted),
            Span::styled(format!("{peer} "), muted),
            Span::styled(s.lost.to_string(), bright),
        ];
        if s.drawn > 0 {
            spans.push(Span::styled(format!("  ·  {} drawn", s.drawn), muted));
        }
        line(buf, y, Line::from(spans));
    }
    y += score_h + 1;

    if let Some((text, tone)) = app.readiness(ctx, &peer) {
        line(buf, y, Line::styled(text, tone_style(tone)));
    }
    y += 1;
    let keys = if app.gone {
        keycaps_fit(&[("esc", "see the boards"), ("q", "leave")], inner.width)
    } else {
        keycaps_fit(
            &[
                ("enter", app.next_label()),
                ("esc", "see the boards"),
                ("q", "leave"),
            ],
            inner.width,
        )
    };
    line(buf, y, Line::from(keys));
}

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Go => Style::default().fg(BRIGHT),
        Tone::Wait | Tone::Done => Style::default().fg(MUTED),
        Tone::Offer => Style::default().fg(CURSOR).add_modifier(Modifier::BOLD),
        Tone::Warn | Tone::Lose => Style::default().fg(CAPTURE),
        Tone::Win => Style::default().fg(SELECTED).add_modifier(Modifier::BOLD),
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx) {
    let chat = ctx.has_chat() && !app.alone();
    let mut keys: Vec<(&str, &str)> = Vec::new();
    if app.card_since().is_some() {
        if !app.gone {
            keys.push(("enter", app.next_label()));
        }
        keys.push(("esc", "see the boards"));
        keys.push(("q", "leave"));
    } else if app.guessing() {
        keys.push(("enter", "guess"));
        keys.push(("⌫", "erase"));
        if chat {
            keys.push(("tab", "chat"));
        }
        keys.push(("esc", "leave"));
    } else {
        if !app.in_play() && !app.gone {
            keys.push(("enter", app.next_label()));
        }
        if app.can_change_mode() {
            keys.push(("h", if app.hard { "normal mode" } else { "hard mode" }));
        }
        if chat {
            keys.push(("t", "chat"));
        }
        if matches!(ctx.conn, Conn::Waiting) {
            keys.push(("c", "copy code"));
        }
        keys.push(("m", "mouse"));
        keys.push(("q", "leave"));
    }
    chrome::footer(f, area, ctx, None, &keys);
}

/// The game's card in the lobby: the last few guesses of a round, marked,
/// ending on the answer.
pub fn thumb(buf: &mut Buffer, area: Rect) {
    const GUESSES: [&str; 4] = ["slate", "crane", "meats", "games"];
    const TILE: u16 = 3;
    let area = area.intersection(buf.area);
    let Some(answer) = super::words::word(GUESSES[GUESSES.len() - 1]) else {
        return;
    };
    let across = cells(LEN) * (TILE + GAP) - GAP;
    // Whatever fits, the answer always among it.
    let shown = GUESSES.len().min(usize::from(area.height));
    let board = centred(area, across, cells(shown));
    for (row, text) in GUESSES[GUESSES.len() - shown..].iter().enumerate() {
        let Some(guess) = super::words::word(text) else {
            continue;
        };
        let marks = super::words::score(&guess, &answer);
        for (col, (&letter, &mark)) in guess.iter().zip(&marks).enumerate() {
            let x = board.x.saturating_add(cells(col) * (TILE + GAP));
            let tile = Rect::new(x, board.y.saturating_add(cells(row)), TILE, 1);
            let look = Look::Marked(mark_colour(mark));
            draw_tile(buf, tile.intersection(area), Some(letter), look, 1.0);
        }
    }
}
