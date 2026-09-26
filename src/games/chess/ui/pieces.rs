//! How a piece is drawn at a given square size: the canvas silhouettes, the
//! half-block sprites, and the character fallbacks under them.

use ratatui::prelude::*;
use shakmaty::{Color as Side, Piece, Role};

use super::board::canvas_dots;
use super::{PieceStyle, centre};
use crate::ui::cells;

/// The rows of text that make up a piece, given how much room the square has.
pub(super) fn piece_rows(role: Role, style: PieceStyle, cell_h: u16) -> Vec<String> {
    if style == PieceStyle::Art && cell_h >= 3 {
        return art(role)
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
    }
    let single = match style {
        PieceStyle::Letter | PieceStyle::BigLetter => letter(role),
        // Art that cannot fit falls back to a figurine rather than vanishing.
        _ => figurine(role),
    };
    vec![single.to_string()]
}

/// A pixel of a sprite: `#` is the outline, `o` the body, anything else lets
/// the square show through.
pub(super) enum Pixel {
    Line,
    Fill,
}

pub(super) struct Sprite {
    rows: &'static [&'static str],
    /// Grows a one-pixel border in the outline colour around whatever is
    /// drawn. Lets a thin letterform read on any square without anyone having
    /// to hand-draw its border.
    outline: bool,
}

impl Sprite {
    const fn solid(rows: &'static [&'static str]) -> Self {
        Self {
            rows,
            outline: false,
        }
    }

    const fn outlined(rows: &'static [&'static str]) -> Self {
        Self {
            rows,
            outline: true,
        }
    }

    fn pad(&self) -> u16 {
        u16::from(self.outline)
    }

    pub(super) fn width(&self) -> u16 {
        cells(self.rows[0].len()) + 2 * self.pad()
    }

    pub(super) fn height(&self) -> u16 {
        cells(self.rows.len()) + 2 * self.pad()
    }

    /// The glyph as written, before any outline is grown around it.
    pub(super) fn raw(&self, x: i32, y: i32) -> Option<Pixel> {
        let row = usize::try_from(y).ok().and_then(|y| self.rows.get(y))?;
        match row.as_bytes().get(usize::try_from(x).ok()?)? {
            b'#' => Some(Pixel::Line),
            b'o' => Some(Pixel::Fill),
            _ => None,
        }
    }

    /// Out-of-range coordinates read as transparent, which is what lets a
    /// sprite be dropped into a larger square without any bounds juggling.
    pub(super) fn at(&self, x: u16, y: u16) -> Option<Pixel> {
        let pad = i32::from(self.pad());
        let (gx, gy) = (i32::from(x) - pad, i32::from(y) - pad);
        if let Some(pixel) = self.raw(gx, gy) {
            return Some(pixel);
        }
        if !self.outline {
            return None;
        }
        let touching = (-1..=1).any(|dy| (-1..=1).any(|dx| self.raw(gx + dx, gy + dy).is_some()));
        touching.then_some(Pixel::Line)
    }
}

/// The colours one side's pieces are drawn in. Both sides are outlined in
/// near-black; black's body is lifted off it far enough that the outline still
/// reads against the body, and stays dark enough to tell from white at a
/// glance.
pub(super) struct Ink {
    pub(super) fill: Color,
    pub(super) line: Color,
}

const WHITE_INK: Ink = Ink {
    fill: Color::Rgb(242, 239, 232),
    line: Color::Rgb(46, 40, 35),
};
const BLACK_INK: Ink = Ink {
    fill: Color::Rgb(68, 60, 54),
    line: Color::Rgb(14, 12, 11),
};

// A letter is all thin strokes, and the grown outline closes up its counters,
// so the body has to carry the contrast against the outline rather than
// against the square. Black's stroke is lifted well clear of the near-black
// border for that, and still reads as the dark side next to white's.
const WHITE_LETTER_INK: Ink = Ink {
    fill: Color::Rgb(245, 242, 236),
    line: Color::Rgb(18, 16, 14),
};
const BLACK_LETTER_INK: Ink = Ink {
    fill: Color::Rgb(128, 115, 103),
    line: Color::Rgb(18, 16, 14),
};

pub(super) fn ink(style: PieceStyle, side: Side) -> &'static Ink {
    match (style, side) {
        (PieceStyle::BigLetter, Side::White) => &WHITE_LETTER_INK,
        (PieceStyle::BigLetter, _) => &BLACK_LETTER_INK,
        (_, Side::White) => &WHITE_INK,
        _ => &BLACK_INK,
    }
}

/// The `(body, outline)` colours a side's pieces are drawn in. Public so a
/// test can read sprites back out of a rendered buffer.
#[must_use]
pub fn piece_ink(style: PieceStyle, side: Side) -> (Color, Color) {
    let ink = ink(style, side);
    (ink.fill, ink.line)
}

/// Nine by eight, drawn inside the 11x10 pixels of an 11x5 square.
fn sprite_big(role: Role) -> Sprite {
    Sprite::solid(match role {
        Role::King => &[
            "....o....",
            "...ooo...",
            "....o....",
            "..#ooo#..",
            ".#ooooo#.",
            "..#ooo#..",
            ".#ooooo#.",
            ".#######.",
        ],
        Role::Queen => &[
            "o.o.o.o.o",
            "#ooooooo#",
            ".#ooooo#.",
            "..#ooo#..",
            "..#ooo#..",
            ".#ooooo#.",
            "#ooooooo#",
            ".#######.",
        ],
        Role::Rook => &[
            ".o.o.o.o.",
            ".#######.",
            ".#ooooo#.",
            "..#ooo#..",
            "..#ooo#..",
            ".#ooooo#.",
            ".#ooooo#.",
            ".#######.",
        ],
        Role::Bishop => &[
            "....o....",
            "...#o#...",
            "..#ooo#..",
            "..#o#o#..",
            "..#ooo#..",
            ".#ooooo#.",
            ".#ooooo#.",
            ".#######.",
        ],
        Role::Knight => &[
            ".....oo..",
            "...#oooo.",
            "..#ooooo#",
            ".#oooooo#",
            "#o#ooooo#",
            "##.#oooo#",
            "...#oooo#",
            ".#######.",
        ],
        Role::Pawn => &[
            ".........",
            "...###...",
            "..#ooo#..",
            "...#o#...",
            "..#ooo#..",
            ".#ooooo#.",
            ".#ooooo#.",
            ".#######.",
        ],
    })
}

/// Seven by seven, drawn inside the 9x8 pixels of a 9x4 square.
fn sprite_mid(role: Role) -> Sprite {
    Sprite::solid(match role {
        Role::King => &[
            "...o...", "..ooo..", "...o...", ".#ooo#.", ".#ooo#.", "#ooooo#", "#######",
        ],
        Role::Queen => &[
            "o.o.o.o", "#ooooo#", ".#ooo#.", ".#ooo#.", ".#ooo#.", "#ooooo#", "#######",
        ],
        Role::Rook => &[
            ".o.o.o.", ".#####.", ".#ooo#.", "..#o#..", "..#o#..", "#ooooo#", "#######",
        ],
        Role::Bishop => &[
            "...o...", "..#o#..", ".#ooo#.", ".#o#o#.", ".#ooo#.", "#ooooo#", "#######",
        ],
        Role::Knight => &[
            "...oo..", ".#oooo#", "#ooooo#", "#o#ooo#", "##.#oo#", "..#ooo#", "#######",
        ],
        Role::Pawn => &[
            "..###..", ".#ooo#.", "..#o#..", ".#ooo#.", ".#ooo#.", "#ooooo#", "#######",
        ],
    })
}

/// Five by five, drawn inside the 7x6 pixels of a 7x3 square.
fn sprite_tiny(role: Role) -> Sprite {
    Sprite::solid(match role {
        Role::King => &["..o..", ".ooo.", "..o..", "#ooo#", "#####"],
        Role::Queen => &["o.o.o", "#ooo#", ".#o#.", "#ooo#", "#####"],
        Role::Rook => &["o.o.o", ".###.", ".#o#.", "#ooo#", "#####"],
        Role::Bishop => &["..o..", ".#o#.", "#o#o#", "#ooo#", "#####"],
        Role::Knight => &["..oo.", ".#oo#", "#ooo#", "##oo#", "#####"],
        Role::Pawn => &[".###.", "#ooo#", ".#o#.", "#ooo#", "#####"],
    })
}

/// The piece's letter as a 5x5 bitmap. An outline is grown around it at draw
/// time, so it occupies 7x7 pixels and reads on either square colour.
fn font(role: Role) -> Sprite {
    Sprite::outlined(match role {
        Role::King => &["o...o", "o..o.", "ooo..", "o..o.", "o...o"],
        Role::Queen => &[".ooo.", "o...o", "o...o", "o..o.", ".oo.o"],
        Role::Rook => &["oooo.", "o...o", "oooo.", "o..o.", "o...o"],
        Role::Bishop => &["oooo.", "o...o", "oooo.", "o...o", "oooo."],
        Role::Knight => &["o...o", "oo..o", "o.o.o", "o..oo", "o...o"],
        Role::Pawn => &["oooo.", "o...o", "oooo.", "o....", "o...."],
    })
}

/// The biggest sprite this square can hold with a margin left around it, if a
/// pixel style was asked for.
pub(super) fn sprite_for(style: PieceStyle, cw: u16, ch: u16, role: Role) -> Option<Sprite> {
    if style == PieceStyle::BigLetter {
        // 7x7 once outlined, so it needs the same room as the mid sprite.
        return (cw >= 9 && ch >= 4).then(|| font(role));
    }
    // The canvas styles hand small squares over to the sprites.
    if !matches!(
        style,
        PieceStyle::Blocks | PieceStyle::Octant | PieceStyle::Braille
    ) {
        return None;
    }
    match (cw, ch) {
        (w, h) if w >= 11 && h >= 5 => Some(sprite_big(role)),
        (w, h) if w >= 9 && h >= 4 => Some(sprite_mid(role)),
        (w, h) if w >= 7 && h >= 3 => Some(sprite_tiny(role)),
        _ => None,
    }
}

/// Where a sprite sits inside its square, in pixels from the square's corner.
/// Centred across, and sunk towards the bottom so the piece stands on the
/// square rather than floating in the middle of it.
pub(super) fn sprite_origin(sprite: &Sprite, cw: u16, ch: u16) -> (u16, u16) {
    (
        (cw - sprite.width()) / 2,
        (2 * ch - sprite.height()).div_ceil(2),
    )
}

/// One square's worth of one piece, on one row of the board.
///
/// Always returns exactly `cw` columns, so callers can lay squares out
/// side by side without measuring.
pub(super) fn piece_cell(
    piece: Piece,
    sub: u16,
    cw: u16,
    ch: u16,
    style: PieceStyle,
    bg: Color,
) -> Vec<Span<'static>> {
    // Canvas pieces are stamped over the board once it is drawn.
    if canvas_dots(style, cw, ch).is_some() {
        return vec![Span::styled(" ".repeat(cw.into()), Style::default().bg(bg))];
    }
    let ink = ink(style, piece.color);

    if let Some(sprite) = sprite_for(style, cw, ch, piece.role) {
        let (x0, y0) = sprite_origin(&sprite, cw, ch);
        let colour = |p: Option<Pixel>| match p {
            Some(Pixel::Line) => ink.line,
            Some(Pixel::Fill) => ink.fill,
            None => bg,
        };

        return (0..cw)
            .map(|x| {
                let sx = x.wrapping_sub(x0);
                let top = sprite.at(sx, (2 * sub).wrapping_sub(y0));
                let bottom = sprite.at(sx, (2 * sub + 1).wrapping_sub(y0));
                if top.is_none() && bottom.is_none() {
                    Span::styled(" ", Style::default().bg(bg))
                } else {
                    // The upper half block paints the top pixel in the
                    // foreground and leaves the bottom one as background.
                    Span::styled("▀", Style::default().fg(colour(top)).bg(colour(bottom)))
                }
            })
            .collect();
    }

    let rows = piece_rows(piece.role, style, ch);
    let top = (ch - cells(rows.len())).div_ceil(2);
    let content = match sub.checked_sub(top) {
        Some(i) if (i as usize) < rows.len() => centre(&rows[i as usize], cw),
        _ => " ".repeat(cw.into()),
    };
    let mut cell = Style::default().fg(ink.fill).bg(bg);
    if piece.color == Side::White {
        cell = cell.add_modifier(Modifier::BOLD);
    }
    vec![Span::styled(content, cell)]
}

fn art(role: Role) -> [&'static str; 3] {
    match role {
        Role::King => ["\\+/", "(K)", "/_\\"],
        Role::Queen => ["\\o/", "(Q)", "/_\\"],
        Role::Rook => ["|-|", "(R)", "/_\\"],
        Role::Bishop => [".^.", "(B)", "/_\\"],
        Role::Knight => ["/^)", "(N)", "/_\\"],
        Role::Pawn => [" o ", "(P)", "/_\\"],
    }
}

fn figurine(role: Role) -> char {
    match role {
        Role::King => '♚',
        Role::Queen => '♛',
        Role::Rook => '♜',
        Role::Bishop => '♝',
        Role::Knight => '♞',
        Role::Pawn => '♟',
    }
}

fn letter(role: Role) -> char {
    match role {
        Role::King => 'K',
        Role::Queen => 'Q',
        Role::Rook => 'R',
        Role::Bishop => 'B',
        Role::Knight => 'N',
        Role::Pawn => 'P',
    }
}
