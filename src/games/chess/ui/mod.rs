//! Drawing a game of chess: the board and its pieces (`board`, `pieces`),
//! the panels around it (`panels`), and the geometry that mouse clicks are
//! tested against.
//!
//! What is drawn and what can be clicked both come from [`Geometry`], so the
//! two cannot drift apart.

mod bar;
mod board;
mod panels;
mod pieces;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;

use shakmaty::{Color as Side, File, Rank, Square};

use bar::draw_bar;
use board::{draw_board, draw_slide};
use panels::{draw_footer, draw_promotion, draw_sidebar, draw_verdict};

use super::app::App;
use super::rules::PROMOTION_ROLES;
use crate::games::{Ctx, chat};
use crate::ui::{cells, centred};

pub use board::canvas_colour;
pub use pieces::piece_ink;

const LIGHT: Color = Color::Rgb(214, 194, 162);
const DARK: Color = Color::Rgb(137, 99, 73);
const LIGHT_LAST: Color = Color::Rgb(206, 204, 122);
const DARK_LAST: Color = Color::Rgb(163, 150, 71);
/// How far the dot on a square you can move to darkens the square beneath.
const MARK_SHADE: f32 = 0.22;
/// A king in check or mated, and the verdict.
const MATE_RED: Color = Color::Rgb(222, 52, 44);
const NIGHT: Color = Color::Rgb(0, 0, 0);
const FLASH: Color = Color::Rgb(255, 255, 255);
const VERDICT_BG: Color = Color::Rgb(22, 16, 14);

/// How a piece is drawn. Each falls back to the next when a square is too
/// small to carry it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieceStyle {
    /// Silhouettes drawn on a [`Canvas`](ratatui::widgets::canvas::Canvas)
    /// in octants: eight solid dots to a cell. See [`super::canvas`].
    Octant,
    /// The same silhouettes in braille, for terminals without octants.
    Braille,
    /// Half-block sprites. Every cell drawn as `▀` holds two stacked pixels —
    /// its foreground on top, its background below — which buys the vertical
    /// resolution a recognisable piece needs.
    Blocks,
    /// The piece's letter as a 5x5 bitmap, blown up through the same half
    /// blocks the sprites use, with an outline grown around it.
    BigLetter,
    /// Three-row line art. Needs a tall square; falls back on its own.
    Art,
    /// A single chess figurine, ♞.
    Figurine,
    /// A single letter, for terminals that render figurines double-width.
    Letter,
}

impl PieceStyle {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            PieceStyle::Blocks => PieceStyle::BigLetter,
            PieceStyle::BigLetter => PieceStyle::Art,
            PieceStyle::Art => PieceStyle::Figurine,
            PieceStyle::Figurine => PieceStyle::Letter,
            PieceStyle::Letter => PieceStyle::Octant,
            PieceStyle::Octant => PieceStyle::Braille,
            PieceStyle::Braille => PieceStyle::Blocks,
        }
    }
}

/// Square sizes we are willing to draw, smallest first. Widths are odd so a
/// single glyph sits dead centre.
const CELL_SIZES: [(u16, u16); 5] = [(3, 1), (5, 2), (7, 3), (9, 4), (11, 5)];
/// Rank digit plus a space, down the left of the board.
const GUTTER: u16 = 2;
const SIDEBAR_MIN: u16 = 24;
/// Past this the sidebar stops growing, and the room left over goes to
/// either side of the board and sidebar instead.
const SIDEBAR_MAX: u16 = 40;
/// The rows of the sidebar that are always there: a card for each player,
/// and the state of the game.
const SIDEBAR_FIXED: u16 = 9;
/// The fewest rows the sidebar needs: that, and a few moves.
const SIDEBAR_MIN_H: u16 = SIDEBAR_FIXED + 3;
/// The chat's column, right of the board, when there is room for it.
const CHAT_MIN: u16 = 24;
const CHAT_MAX: u16 = 48;
/// The fewest rows the chat can do with, when there is no room beside the
/// board and it goes under the moves instead: a message and the composer.
const CHAT_MIN_H: u16 = 6;

/// The evaluation bar's column, left of the board: a space either side of a
/// bar three wide, which is room for a score like `1.3` or `M4`.
const BAR_W: u16 = 5;

/// Where everything sits this frame.
pub struct Geometry {
    pub board: Rect,
    /// The 8x8 playing area, inside the border and to the right of the gutter.
    pub grid: Rect,
    pub cell: (u16, u16),
    /// The game and its moves, left of the board.
    pub sidebar: Rect,
    /// Right of the board where there is room, else under the moves. `None`
    /// in hot-seat, and on a screen too small for it.
    pub chat: Option<Rect>,
    /// The evaluation bar, beside the ranks, while analysis is on.
    pub eval: Option<Rect>,
    pub footer: Rect,
    pub promo: Rect,
    pub promo_cell: u16,
}

impl Geometry {
    /// The layout for a game with nobody to talk to.
    #[must_use]
    pub fn new(area: Rect) -> Self {
        Self::layout(area, false, false)
    }

    /// The layout for the game at this table: with the chat, unless it is
    /// hot-seat.
    pub fn of(area: Rect, ctx: &Ctx) -> Self {
        Self::layout(area, ctx.has_chat(), false)
    }

    /// The layout for this game at this table: with the chat, unless it is
    /// hot-seat, and the evaluation bar while analysis is on.
    pub fn for_game(area: Rect, ctx: &Ctx, app: &App) -> Self {
        Self::layout(area, ctx.has_chat(), app.engine.is_some())
    }

    /// Picks the biggest board that leaves room for the sidebar, and centres
    /// it on the screen. The sidebar sits to its left, or pushes it right of
    /// centre where there is not room for both. The chat, if `chat`, takes
    /// the room to the board's right; the board is never shrunk for it, so
    /// where there is not room it goes under the moves instead. The bar, if
    /// `bar`, goes between the sidebar and the board.
    #[must_use]
    pub fn layout(area: Rect, chat: bool, bar: bool) -> Self {
        let bar_w = if bar { BAR_W } else { 0 };
        let [main, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

        let mut cell = CELL_SIZES[0];
        for candidate in CELL_SIZES {
            let (w, h) = (block_w(candidate.0), block_h(candidate.1));
            if w + bar_w + SIDEBAR_MIN <= main.width && h <= main.height {
                cell = candidate;
            }
        }
        let (cw, ch) = cell;

        // The board, with the bar beside it.
        let (col_w, col_h) = (
            (block_w(cw) + bar_w).min(main.width),
            block_h(ch).min(main.height),
        );
        let spare = main.width - col_w;
        let beside = chat && spare >= SIDEBAR_MIN + CHAT_MIN;
        let (side_w, chat_w) = if beside {
            // Both columns get their minimum, then share what is left.
            let rest = spare - SIDEBAR_MIN - CHAT_MIN;
            let side_more = (rest / 2).min(SIDEBAR_MAX - SIDEBAR_MIN);
            let chat_more = (rest - side_more).min(CHAT_MAX - CHAT_MIN);
            (SIDEBAR_MIN + side_more, CHAT_MIN + chat_more)
        } else {
            (spare.min(SIDEBAR_MAX), 0)
        };
        // The board in the middle of the screen, if the columns either side
        // leave it there.
        let board_x = (spare / 2).max(side_w).min(spare - chat_w);
        let left = Rect {
            x: main.x + board_x,
            y: main.y + (main.height - col_h) / 2,
            width: col_w,
            height: col_h,
        };
        let side_h = col_h.max(SIDEBAR_MIN_H).min(main.height);
        let mut sidebar = Rect {
            x: left.x - side_w,
            y: main.y + (main.height - side_h) / 2,
            width: side_w,
            height: side_h,
        };
        let chat = if beside {
            Some(Rect {
                x: left.right(),
                width: chat_w,
                ..sidebar
            })
        } else if chat && side_w >= SIDEBAR_MIN && side_h >= SIDEBAR_MIN_H + CHAT_MIN_H {
            // Under the moves, taking half of what the cards leave.
            let below = ((side_h - SIDEBAR_FIXED) / 2)
                .max(CHAT_MIN_H)
                .min(side_h - SIDEBAR_MIN_H);
            sidebar.height -= below;
            Some(Rect {
                y: sidebar.bottom(),
                height: below,
                ..sidebar
            })
        } else {
            None
        };
        let board_w = col_w.saturating_sub(bar_w);
        let board = Rect {
            x: left.x + (col_w - board_w),
            width: board_w,
            ..left
        };

        let grid = Rect {
            x: board.x + 1 + GUTTER,
            y: board.y + 1,
            width: 8 * cw,
            height: 8 * ch,
        };

        // Level with the ranks, so its middle is the middle of the board.
        let eval = (bar && board.x >= left.x + BAR_W).then(|| {
            Rect {
                x: left.x + 1,
                y: grid.y,
                width: BAR_W - 2,
                height: 8 * ch,
            }
            .intersection(main)
        });

        // The prompt shows four pieces at roughly board scale.
        let promo_cell = cw.clamp(5, 9);
        let promo = centred(board, 4 * promo_cell + 2, ch.clamp(3, 4) + 2);

        Self {
            board,
            grid,
            cell,
            sidebar,
            chat,
            eval,
            footer,
            promo,
            promo_cell,
        }
    }

    /// The square under a screen position, if any.
    #[must_use]
    pub fn square_at(&self, x: u16, y: u16, flipped: bool) -> Option<Square> {
        let (cw, ch) = self.cell;
        let col = x.checked_sub(self.grid.x)? / cw;
        let row = y.checked_sub(self.grid.y)? / ch;
        if col > 7 || row > 7 {
            return None;
        }
        let file = if flipped { 7 - col } else { col };
        let rank = if flipped { row } else { 7 - row };
        Some(Square::from_coords(
            File::new(file.into()),
            Rank::new(rank.into()),
        ))
    }

    /// The promotion choice under a screen position, if any.
    #[must_use]
    pub fn promo_at(&self, x: u16, y: u16) -> Option<usize> {
        if y <= self.promo.y || y + 1 >= self.promo.bottom() {
            return None;
        }
        let index = (x.checked_sub(self.promo.x + 1)? / self.promo_cell) as usize;
        (index < PROMOTION_ROLES.len()).then_some(index)
    }
}

fn block_w(cell_w: u16) -> u16 {
    8 * cell_w + GUTTER + 2
}

/// Eight ranks, the file labels, and the border.
fn block_h(cell_h: u16) -> u16 {
    8 * cell_h + 1 + 2
}

pub fn draw(f: &mut Frame, app: &App, ctx: &Ctx) {
    let g = Geometry::for_game(f.area(), ctx, app);

    // Looking back draws that position, still; otherwise the game as it is.
    let reviewed = app.reviewed();
    let live = reviewed.is_none();
    draw_board(
        f,
        &g,
        app,
        reviewed.as_ref().unwrap_or(&app.game),
        live,
        app.best_move(),
    );
    if live && let Some((slide, t)) = app.slide_at() {
        draw_slide(f.buffer_mut(), &g, app, slide, t);
    }
    if let Some(area) = g.eval {
        draw_bar(f.buffer_mut(), area, app);
    }
    draw_sidebar(f, g.sidebar, app, ctx);
    if let Some(area) = g.chat {
        chat::draw(f, area, ctx);
    }
    draw_footer(f, g.footer, app, ctx);

    if app.game.promotion.is_some() {
        draw_promotion(f, &g, app);
    }
    if live
        && let Some(fin) = app.finale()
        && let Some(reveal) = fin.banner
    {
        draw_verdict(f, &g, app, ctx, &fin, reveal);
    }
}

/// The game's card in the lobby: a few squares of a board with pieces on
/// them, drawn as silhouettes when there is room and as figurines when not.
pub fn thumb(buf: &mut Buffer, area: Rect) {
    use shakmaty::{Piece, Role};
    const SHOWN: [(Role, Side); 3] = [
        (Role::Knight, Side::White),
        (Role::Queen, Side::Black),
        (Role::King, Side::White),
    ];
    let area = area.intersection(buf.area);
    // Squares about twice as wide as tall look square; at 9x4 the canvas
    // pieces keep their detail, and two of them still say chess.
    let big = area.height >= 4 && area.width >= 2 * 9;
    let cell = if big { (9, 4) } else { (3, 1) };
    let n = SHOWN.len().min(usize::from(area.width / cell.0));
    let board = centred(area, cell.0 * cells(n), cell.1);
    let mut pieces = Vec::new();
    for (i, &(role, color)) in SHOWN[..n].iter().enumerate() {
        let col = cells(i);
        let square = Rect {
            x: board.x + col * cell.0,
            width: cell.0,
            ..board
        }
        .intersection(board);
        let bg = if i % 2 == 0 { LIGHT } else { DARK };
        for y in square.top()..square.bottom() {
            for x in square.left()..square.right() {
                buf[(x, y)].set_symbol(" ").set_bg(bg);
            }
        }
        let piece = Piece { color, role };
        if big {
            pieces.push(board::canvas_piece(piece, (col, 0), (0.0, 0.0), cell));
        } else if !square.is_empty() {
            let at = (square.x + square.width / 2, square.y + square.height / 2);
            buf[at]
                .set_char(piece.char().to_ascii_uppercase())
                .set_fg(canvas_colour(color));
        }
    }
    if big {
        super::canvas::stamp(buf, board, &pieces, super::canvas::Dots::Octant);
    }
}

/// Pads `s` to `width` columns with the content centred.
fn centre(s: &str, width: u16) -> String {
    let width = usize::from(width);
    let len = s.chars().count();
    if len >= width {
        return s.chars().take(width).collect();
    }
    let left = (width - len) / 2;
    format!(
        "{}{}{}",
        " ".repeat(left),
        s,
        " ".repeat(width - len - left)
    )
}

fn side_name(c: Side) -> &'static str {
    if c == Side::White { "white" } else { "black" }
}
