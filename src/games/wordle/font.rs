//! Big letters and numbers, drawn in half blocks so each cell holds two
//! pixels, one above the other.
//!
//! Letters use the classic 5x7 font of character displays, where every
//! letter stays distinct: five cells wide and four rows tall. Numbers on the
//! score card use a 3x5 font, which is plenty for digits and takes a third
//! of the room.

use ratatui::buffer::Buffer;
use ratatui::style::Style;

/// Which font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// 3x5: digits and `%` only.
    Small,
    /// 5x7: letters, digits and a little punctuation.
    Large,
}

/// Rows of five pixels, top first, the leftmost pixel the highest bit.
const LARGE: [(char, [u8; 7]); 38] = [
    ('A', [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
    ('B', [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
    ('C', [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E]),
    ('D', [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E]),
    ('E', [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F]),
    ('F', [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10]),
    ('G', [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F]),
    ('H', [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
    ('I', [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('J', [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C]),
    ('K', [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
    ('L', [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F]),
    ('M', [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11]),
    ('N', [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11]),
    ('O', [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
    ('P', [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10]),
    ('Q', [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D]),
    ('R', [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11]),
    ('S', [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E]),
    ('T', [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('U', [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
    ('V', [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04]),
    ('W', [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A]),
    ('X', [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11]),
    ('Y', [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04]),
    ('Z', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F]),
    ('0', [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
    ('1', [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('2', [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F]),
    ('3', [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E]),
    ('4', [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02]),
    ('5', [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E]),
    ('6', [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E]),
    ('7', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
    ('8', [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
    ('9', [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C]),
    ('%', [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03]),
    ('!', [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04]),
];

/// Rows of three pixels, top first.
const SMALL: [(char, [u8; 5]); 11] = [
    ('0', [0b111, 0b101, 0b101, 0b101, 0b111]),
    ('1', [0b010, 0b110, 0b010, 0b010, 0b111]),
    ('2', [0b111, 0b001, 0b111, 0b100, 0b111]),
    ('3', [0b111, 0b001, 0b111, 0b001, 0b111]),
    ('4', [0b101, 0b101, 0b111, 0b001, 0b001]),
    ('5', [0b111, 0b100, 0b111, 0b001, 0b111]),
    ('6', [0b111, 0b100, 0b111, 0b101, 0b111]),
    ('7', [0b111, 0b001, 0b001, 0b001, 0b001]),
    ('8', [0b111, 0b101, 0b111, 0b101, 0b111]),
    ('9', [0b111, 0b101, 0b111, 0b001, 0b111]),
    ('%', [0b101, 0b001, 0b010, 0b100, 0b101]),
];

impl Size {
    /// Pixels across a glyph.
    fn pixels(self) -> u16 {
        match self {
            Size::Small => 3,
            Size::Large => 5,
        }
    }

    /// Rows a glyph takes on screen: two pixels to a row.
    #[must_use]
    pub fn height(self) -> u16 {
        match self {
            Size::Small => 3,
            Size::Large => 4,
        }
    }

    /// Columns `text` takes, with one between each glyph.
    #[must_use]
    pub fn width(self, text: &str) -> u16 {
        let n = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
        n.saturating_mul(self.pixels() + 1).saturating_sub(1)
    }

    /// The pixel rows of `c`, top first, blank for anything the font lacks.
    /// The small font's five rows are padded out to seven.
    fn glyph(self, c: char) -> [u8; 7] {
        let c = c.to_ascii_uppercase();
        match self {
            Size::Small => SMALL
                .iter()
                .find(|(g, _)| *g == c)
                .map_or([0; 7], |(_, r)| [r[0], r[1], r[2], r[3], r[4], 0, 0]),
            Size::Large => LARGE
                .iter()
                .find(|(g, _)| *g == c)
                .map_or([0; 7], |(_, rows)| *rows),
        }
    }

    /// The half block for column `col` of cell row `row` of `glyph`: its top
    /// pixel from one pixel row, its bottom from the next.
    fn cell(self, glyph: [u8; 7], row: usize, col: u16) -> char {
        let bit = 1 << (self.pixels() - 1 - col);
        let top = glyph[row * 2] & bit != 0;
        let bottom = glyph.get(row * 2 + 1).is_some_and(|r| r & bit != 0);
        match (top, bottom) {
            (true, true) => '█',
            (true, false) => '▀',
            (false, true) => '▄',
            (false, false) => ' ',
        }
    }

    /// The rows of `text` on screen, a string each.
    #[must_use]
    pub fn rows(self, text: &str) -> Vec<String> {
        let mut out = vec![String::new(); usize::from(self.height())];
        for (n, c) in text.chars().enumerate() {
            let glyph = self.glyph(c);
            for (row, line) in out.iter_mut().enumerate() {
                if n > 0 {
                    line.push(' ');
                }
                line.extend((0..self.pixels()).map(|col| self.cell(glyph, row, col)));
            }
        }
        out
    }

    /// Draws `text` with its top left at `(x, y)`, clipped to the buffer.
    /// Only the pixels are drawn, so whatever is behind them shows.
    pub fn draw(self, buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
        let area = buf.area;
        for (n, c) in (0u16..).zip(text.chars()) {
            let glyph = self.glyph(c);
            let left = x.saturating_add(n * (self.pixels() + 1));
            for row in 0..self.height() {
                let at_y = y.saturating_add(row);
                for col in 0..self.pixels() {
                    let at_x = left.saturating_add(col);
                    let ch = self.cell(glyph, usize::from(row), col);
                    if ch != ' ' && area.contains((at_x, at_y).into()) {
                        buf[(at_x, at_y)].set_char(ch).set_style(style);
                    }
                }
            }
        }
    }
}
