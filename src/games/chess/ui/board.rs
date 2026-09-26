//! The board itself: its squares, the marks on them, the pieces standing on
//! them, and a piece on its way from one to another.

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Paragraph};
use shakmaty::{Color as Side, File, Move, Piece, Rank, Square};

use super::pieces::{Pixel, ink, piece_cell, sprite_for, sprite_origin};
use super::{
    DARK, DARK_LAST, FLASH, GUTTER, Geometry, LIGHT, LIGHT_LAST, MARK_SHADE, MATE_RED, NIGHT,
    PieceStyle, centre,
};
use crate::games::chess::app::{App, Finale, Slide};
use crate::games::chess::canvas::{self, Dots};
use crate::games::chess::rules::{Game, ui_to};
use crate::ui::{CURSOR, MUTED, SELECTED, blend};

/// The board as `game` has it. `live` is the game being played, with its
/// cursor, the moves the piece picked up can make, and whatever is moving;
/// anything else is a position being looked back at, drawn still. `hint` is
/// a move to point out: the engine's best.
/// The engine's best move, pointed out in blue.
const HINT: Color = Color::Rgb(72, 146, 214);

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the board, which reads best in one place"
)]
pub(super) fn draw_board(
    f: &mut Frame,
    g: &Geometry,
    app: &App,
    game: &Game,
    live: bool,
    hint: Option<Move>,
) {
    let (cw, ch) = g.cell;
    let targets = if live { game.targets() } else { Vec::new() };
    let style = app.piece_style;
    let mut lines = Vec::with_capacity(usize::from(8 * ch + 1));

    // Whatever is sliding is drawn on top afterwards, not in place.
    let travelling = app.slide_at().filter(|_| live).map(|(s, _)| s.to);
    let finale = app.finale().filter(|_| live);
    let check = app.check_glow().filter(|_| live);
    let hint = hint.map(|m| (m.from().unwrap_or_else(|| m.to()), ui_to(m)));

    for row in 0..8u32 {
        let rank = if app.flipped { row } else { 7 - row };

        for sub in 0..ch {
            // The rank digit goes on the row the pieces' middles sit on.
            let label = if sub == ch / 2 {
                format!("{} ", Rank::new(rank).char())
            } else {
                " ".repeat(GUTTER.into())
            };
            let mut spans = vec![Span::styled(label, Style::default().fg(MUTED))];

            for col in 0..8u32 {
                let file = if app.flipped { 7 - col } else { col };
                let sq = Square::from_coords(File::new(file), Rank::new(rank));
                let piece = if travelling == Some(sq) {
                    None
                } else {
                    game.piece_at(sq)
                };
                let dark = (file + rank) % 2 == 0;
                let mut bg = match (dark, game.last.is_some_and(|(a, b)| a == sq || b == sq)) {
                    (true, false) => DARK,
                    (false, false) => LIGHT,
                    (true, true) => DARK_LAST,
                    (false, true) => LIGHT_LAST,
                };
                if let Some((from, to)) = hint {
                    if sq == from {
                        bg = blend(bg, HINT, 0.35);
                    } else if sq == to {
                        bg = blend(bg, HINT, 0.6);
                    }
                }
                if game.selected == Some(sq) {
                    bg = blend(bg, SELECTED, 0.8);
                }
                if live && sq == game.cursor {
                    bg = blend(bg, CURSOR, 0.5);
                }
                if let Some((king, glow)) = check
                    && sq == king
                {
                    bg = blend(bg, MATE_RED, glow);
                }
                if let Some(fin) = &finale {
                    bg = finale_bg(fin, sq, bg);
                }

                match piece {
                    Some(p) => spans.extend(piece_cell(p, sub, cw, ch, style, bg)),
                    None => {
                        spans.push(Span::styled(" ".repeat(cw.into()), Style::default().bg(bg)));
                    }
                }
            }
            lines.push(Line::from(spans));
        }
    }

    let mut labels = vec![Span::raw(" ".repeat(GUTTER.into()))];
    for col in 0..8u32 {
        let file = if app.flipped { 7 - col } else { col };
        labels.push(Span::styled(
            centre(&File::new(file).char().to_string(), cw),
            Style::default().fg(MUTED),
        ));
    }
    lines.push(Line::from(labels));

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
        .title(Line::from(" chess ").centered());
    f.render_widget(Paragraph::new(lines).block(block), g.board);

    if let Some(dots) = canvas_dots(style, cw, ch) {
        let mut pieces = Vec::new();
        for row in 0..8u16 {
            for col in 0..8u16 {
                let sq = screen_square(col, row, app.flipped);
                let Some(piece) = game.piece_at(sq).filter(|_| travelling != Some(sq)) else {
                    continue;
                };
                let mut drawn = canvas_piece(piece, (col, row), (0.0, 0.0), g.cell);
                if let Some(fin) = &finale {
                    if sq == fin.king {
                        drawn.at.0 += fin.shake;
                        drawn.fallen = fin.fallen;
                    } else if !fin.checkers.contains(&sq) {
                        // Everything but the king and its killers fades back
                        // with the board.
                        drawn.colour = blend(drawn.colour, NIGHT, fin.dim * 0.6);
                    }
                }
                pieces.push(drawn);
            }
        }
        canvas::stamp(f.buffer_mut(), g.grid, &pieces, dots);
    }

    let dots = (style == PieceStyle::Octant).then_some(Dots::Octant);
    for sq in targets {
        if travelling == Some(sq) {
            continue;
        }
        let capture = game.piece_at(sq).is_some();
        draw_mark(f.buffer_mut(), g, app.flipped, sq, capture, dots);
    }
}

/// Marks a square the selected piece can move to: a dot in the middle of an
/// empty square, or its corners filled in round a circle if there is a piece
/// there to capture. A ring would run through the piece, which fills its
/// square top to bottom; the corners are always free.
///
/// The mark is drawn in octants where the terminal has them and half blocks
/// where it may not, a shade darker than the square. It leaves alone any cell
/// a piece is already drawn in.
#[expect(
    clippy::many_single_char_names,
    reason = "screen geometry, named as it is everywhere else here"
)]
fn draw_mark(
    buf: &mut Buffer,
    g: &Geometry,
    flipped: bool,
    sq: Square,
    capture: bool,
    dots: Option<Dots>,
) {
    let (cw, ch) = g.cell;
    let (file, rank) = (sq.file() as u16, sq.rank() as u16);
    let (col, row) = if flipped {
        (7 - file, rank)
    } else {
        (file, 7 - rank)
    };
    let (sx, sy) = match dots {
        Some(_) => canvas::DOTS,
        None => (1, 2),
    };

    // Measured in cell widths, taking a cell to be twice as tall as it is
    // wide, so the mark comes out round rather than squashed.
    let (w, h) = (f64::from(cw), 2.0 * f64::from(ch));
    let size = w.min(h);
    let pixel = (1.0 / f64::from(sx)).max(2.0 / f64::from(sy));
    let inside = |x: f64, y: f64| {
        let d = (x - w / 2.0).hypot(y - h / 2.0);
        if capture {
            d >= 0.58 * size
        } else {
            d <= (0.18 * size).max(0.6 * pixel)
        }
    };

    for cy in 0..ch {
        for cx in 0..cw {
            let at = (g.grid.x + col * cw + cx, g.grid.y + row * ch + cy);
            if !buf.area.contains(at.into()) || buf[at].symbol() != " " {
                continue;
            }
            let mut bits = 0u8;
            for j in 0..sy {
                for i in 0..sx {
                    let x = f64::from(cx) + (f64::from(i) + 0.5) / f64::from(sx);
                    let y = 2.0 * (f64::from(cy) + (f64::from(j) + 0.5) / f64::from(sy));
                    if inside(x, y) {
                        bits |= 1 << (i + sx * j);
                    }
                }
            }
            if bits == 0 {
                continue;
            }
            let symbol = match dots {
                Some(dots) => dots.encode(bits),
                None => [' ', '▀', '▄', '█'][usize::from(bits)],
            };
            let cell = &mut buf[at];
            let shade = blend(cell.bg, NIGHT, MARK_SHADE);
            cell.set_char(symbol).set_fg(shade);
        }
    }
}

/// A square's background during the checkmate finale: the king's square
/// burns red, the checkers' squares glow with it, and the rest goes dark.
fn finale_bg(fin: &Finale, sq: Square, bg: Color) -> Color {
    let bg = if sq == fin.king {
        blend(bg, MATE_RED, fin.red)
    } else if fin.checkers.contains(&sq) {
        blend(bg, MATE_RED, 0.3)
    } else {
        blend(bg, NIGHT, fin.dim)
    };
    blend(bg, FLASH, fin.impact)
}

/// Which dots to draw the pieces in, if a canvas style is in use and the
/// square has room for one. Below 9x4 the detail goes (the king's cross
/// shrinks to a dot), so smaller squares get the half-block sprites instead.
pub(super) fn canvas_dots(style: PieceStyle, cw: u16, ch: u16) -> Option<Dots> {
    let dots = match style {
        PieceStyle::Octant => Dots::Octant,
        PieceStyle::Braille => Dots::Braille,
        _ => return None,
    };
    (cw >= 9 && ch >= 4).then_some(dots)
}

/// The square at a column and row of the grid as it is drawn.
fn screen_square(col: u16, row: u16, flipped: bool) -> Square {
    let (col, row) = (u32::from(col), u32::from(row));
    let file = if flipped { 7 - col } else { col };
    let rank = if flipped { row } else { 7 - row };
    Square::from_coords(File::new(file), Rank::new(rank))
}

/// A canvas piece in the square at `(col, row)` of a grid of `cell`-sized
/// squares, nudged by `offset` dots.
pub(super) fn canvas_piece(
    piece: Piece,
    (col, row): (u16, u16),
    offset: (f64, f64),
    cell: (u16, u16),
) -> canvas::Piece {
    let square = (cell.0 * canvas::DOTS.0, cell.1 * canvas::DOTS.1);
    canvas::Piece {
        role: piece.role,
        colour: canvas_colour(piece.color),
        at: (
            f64::from(col * square.0) + offset.0,
            f64::from(row * square.1) + offset.1,
        ),
        square,
        fallen: 0.0,
    }
}

/// The colour a side's canvas pieces are drawn in. Public so a test can
/// read the pieces back out of a rendered buffer.
#[must_use]
pub fn canvas_colour(side: Side) -> Color {
    match side {
        Side::White => Color::Rgb(255, 255, 255),
        Side::Black => Color::Rgb(20, 18, 16),
    }
}

/// Draws the travelling piece over the board it was already drawn onto.
///
/// Working straight on the buffer keeps the piece free of the square grid, so
/// it can sit halfway between two squares.
#[expect(
    clippy::many_single_char_names,
    reason = "screen geometry, named as it is everywhere else here"
)]
pub(super) fn draw_slide(buf: &mut Buffer, g: &Geometry, app: &App, slide: &Slide, t: f32) {
    let (cw, ch) = g.cell;
    // Ease in and out, so the piece does not start and stop abruptly.
    let e = t * t * (3.0 - 2.0 * t);

    if let Some(dots) = canvas_dots(app.piece_style, cw, ch) {
        // Canvas pieces move a dot at a time, and may stop between dots.
        let place = |sq: Square| {
            let (file, rank) = (sq.file() as u16, sq.rank() as u16);
            if app.flipped {
                (7 - file, rank)
            } else {
                (file, 7 - rank)
            }
        };
        let (from, to) = (place(slide.from), place(slide.to));
        let square = (cw * canvas::DOTS.0, ch * canvas::DOTS.1);
        let offset = (
            f64::from(e) * (f64::from(to.0) - f64::from(from.0)) * f64::from(square.0),
            f64::from(e) * (f64::from(to.1) - f64::from(from.1)) * f64::from(square.1),
        );
        let piece = canvas_piece(slide.piece, from, offset, g.cell);
        canvas::stamp(buf, g.grid, &[piece], dots);
        return;
    }

    // Character styles have nothing to interpolate; they just arrive.
    let Some(sprite) = sprite_for(app.piece_style, cw, ch, slide.piece.role) else {
        return;
    };
    let (x0, y0) = sprite_origin(&sprite, cw, ch);

    // A square's sprite origin, in pixels from the grid's top-left corner.
    let origin = |sq: Square| {
        let (file, rank) = (i32::from(sq.file() as u8), i32::from(sq.rank() as u8));
        let (col, row) = if app.flipped {
            (7 - file, rank)
        } else {
            (file, 7 - rank)
        };
        (
            col * i32::from(cw) + i32::from(x0),
            row * 2 * i32::from(ch) + i32::from(y0),
        )
    };
    let (fx, fy) = origin(slide.from);
    let (tx, ty) = origin(slide.to);

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "a distance across the board, in cells, is small enough to go through f32 and back"
    )]
    let lerp = |a: i32, b: i32| a + (((b - a) as f32) * e).round() as i32;
    let (x, y) = (lerp(fx, tx), lerp(fy, ty));

    let ink = ink(app.piece_style, slide.piece.color);
    let colour = |p: Pixel| match p {
        Pixel::Line => ink.line,
        Pixel::Fill => ink.fill,
    };

    let rows = i32::from(sprite.height());
    for cy in y.div_euclid(2)..=(y + rows - 1).div_euclid(2) {
        let Ok(row) = u16::try_from(cy) else { continue };
        if row >= g.grid.height {
            continue;
        }
        for cx in x..x + i32::from(sprite.width()) {
            let Ok(col) = u16::try_from(cx) else { continue };
            if col >= g.grid.width {
                continue;
            }
            let Ok(sx) = u16::try_from(cx - x) else {
                continue;
            };
            let pixel = |py: i32| u16::try_from(py - y).ok().and_then(|sy| sprite.at(sx, sy));
            let (top, bottom) = (pixel(cy * 2), pixel(cy * 2 + 1));
            if top.is_none() && bottom.is_none() {
                continue;
            }

            // Keep whatever the board already put behind the transparent half.
            let pos = (g.grid.x + col, g.grid.y + row);
            let cell = &buf[pos];
            let (was_top, was_bottom) = if cell.symbol() == "▀" {
                (cell.fg, cell.bg)
            } else {
                (cell.bg, cell.bg)
            };
            let fg = top.map_or(was_top, colour);
            let bg = bottom.map_or(was_bottom, colour);
            buf[pos].set_symbol("▀").set_fg(fg).set_bg(bg);
        }
    }
}
