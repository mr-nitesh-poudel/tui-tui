//! Pieces drawn on ratatui's [`Canvas`], two dots across and four down to a
//! cell.
//!
//! That grid comes in two kinds of character. Octants fill each dot in solid,
//! so the dots run together into one shape; braille draws each as a small
//! round dot with a gap around it. Octants are new to Unicode (16.0) and not
//! every terminal draws them yet, which is why braille stays as the fallback.
//!
//! Either way a square of `w`x`h` cells becomes a `2w`x`4h` bitmap: 22x20 on
//! the biggest board, where the half-block sprites get 11x10 pixels. Terminal
//! cells are about twice as tall as they are wide, which makes the dots close
//! to square, so the pieces keep their proportions.
//!
//! The pieces are not bitmaps but silhouettes, built from circles, ellipses
//! and polygons in a unit box and sampled at each dot. That is what lets one
//! drawing serve every square size, and a travelling piece sit between dots
//! rather than jump from cell to cell.
//!
//! Every dot in a cell shares one colour, so a piece is one solid colour too:
//! white for white, near-black for black.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::symbols::Marker;
use ratatui::symbols::pixel::OCTANTS;
use ratatui::widgets::Widget;
use ratatui::widgets::canvas::{Canvas, Painter, Shape};
use shakmaty::Role;

/// Dots per cell, across and down.
pub const DOTS: (u16, u16) = (2, 4);

/// Empty dots left between a piece and the top and sides of its square.
const PADDING: f64 = 1.0;
/// And below it, where a little more room keeps a piece from looking as if it
/// sits on the square beneath.
const PADDING_BELOW: f64 = 2.0;

/// Which characters the dots are drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dots {
    /// Solid blocks that run together.
    Octant,
    /// Round dots with gaps between them, which every terminal can draw.
    Braille,
}

/// For each dot, numbered `x + 2y`, its bit in a braille character.
const BRAILLE_BIT: [u8; 8] = [0x01, 0x08, 0x02, 0x10, 0x04, 0x20, 0x40, 0x80];

impl Dots {
    fn marker(self) -> Marker {
        match self {
            Dots::Octant => Marker::Octant,
            Dots::Braille => Marker::Braille,
        }
    }

    /// The dots a cell's symbol shows, as bit `x + 2y` for the dot at
    /// `(x, y)`. `None` if it is not one of this kind's characters. Public so
    /// a test can read pieces back out of a rendered buffer.
    #[must_use]
    pub fn decode(self, symbol: &str) -> Option<u8> {
        let mut chars = symbol.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        match self {
            // Blank as well as dotted cells, so an empty square decodes too.
            Dots::Octant => OCTANTS
                .iter()
                .position(|&o| o == c)
                .and_then(|i| u8::try_from(i).ok()),
            Dots::Braille => {
                let raw = u8::try_from(u32::from(c).checked_sub(0x2800)?).ok()?;
                Some(
                    (0..8)
                        .filter(|&k| raw & BRAILLE_BIT[k] != 0)
                        .fold(0, |bits, k| bits | 1 << k),
                )
            }
        }
    }

    /// The character that shows the dots in `bits`, numbered as in [`Dots::decode`].
    #[must_use]
    pub fn encode(self, bits: u8) -> char {
        match self {
            Dots::Octant => OCTANTS[usize::from(bits)],
            Dots::Braille => {
                let raw = (0..8)
                    .filter(|&k| bits & 1 << k != 0)
                    .fold(0, |raw, k| raw | u32::from(BRAILLE_BIT[k]));
                char::from_u32(0x2800 | raw).unwrap_or(' ')
            }
        }
    }
}

/// One piece, placed in dot coordinates within the canvas it is drawn on.
pub struct Piece {
    pub role: Role,
    pub colour: Color,
    /// The top-left dot of the square the piece stands in. Fractional while
    /// it is travelling.
    pub at: (f64, f64),
    /// The square's size in dots.
    pub square: (u16, u16),
    /// Toppled over: 0 standing, 1 lying on its right side, -1 on its left.
    pub fallen: f64,
}

impl Piece {
    /// Where a dot in the square's box falls on the piece, as (x, y) in the
    /// unit box with y pointing up. The box is square and centred across,
    /// with [`PADDING`] dots clear above and at the sides and
    /// [`PADDING_BELOW`] below, so no piece touches its square's edge.
    fn unit(&self, dx: f64, dy: f64) -> (f64, f64) {
        let (w, h) = (f64::from(self.square.0), f64::from(self.square.1));
        let size = (h - PADDING - PADDING_BELOW).min(w - 2.0 * PADDING);
        let left = (w - size) / 2.0;
        let bottom = h - PADDING_BELOW;
        ((dx + 0.5 - left) / size, (bottom - (dy + 0.5)) / size)
    }

    /// Where a point of the square's unit box falls on the piece once it has
    /// toppled: turned about its middle, and let down as it turns so that it
    /// ends up lying on the floor of the square rather than floating.
    fn upright(&self, x: f64, y: f64) -> (f64, f64) {
        if self.fallen == 0.0 {
            return (x, y);
        }
        let angle = self.fallen * std::f64::consts::FRAC_PI_2;
        // Lying down, the widest part (the foot, 0.3 either side of the
        // middle) is what rests on the floor.
        let drop = 0.18 * angle.sin().abs();
        let (qx, qy) = (x - 0.5, y + drop - 0.5);
        let (sin, cos) = angle.sin_cos();
        (qx * cos - qy * sin + 0.5, qx * sin + qy * cos + 0.5)
    }
}

impl Shape for Piece {
    fn draw(&self, painter: &mut Painter) {
        let (x_bounds, y_bounds) = painter.bounds();
        let (w, h) = (i32::from(self.square.0), i32::from(self.square.1));
        let (fx, fy) = (self.at.0 - self.at.0.floor(), self.at.1 - self.at.1.floor());
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the canvas bounds and a piece's place on it are whole dots, well within range"
        )]
        let ((width, height), (ax, ay)) = (
            (x_bounds[1] as usize + 1, y_bounds[1] as usize + 1),
            (self.at.0.floor() as i32, self.at.1.floor() as i32),
        );

        // Sample the silhouette on the square's own dot grid, offset by the
        // fraction of a dot the piece has travelled.
        let solid = |x: i32, y: i32| {
            let (ux, uy) = self.unit(f64::from(x) - fx, f64::from(y) - fy);
            let (ux, uy) = self.upright(ux, uy);
            inside(self.role, ux, uy)
        };

        // Halfway over, a toppling piece reaches out past its square.
        let reach = if self.fallen == 0.0 { 1 } else { h / 2 };
        for y in -reach..=h + reach {
            for x in -reach..=w + reach {
                if !solid(x, y) {
                    continue;
                }
                let (Ok(px), Ok(py)) = (usize::try_from(ax + x), usize::try_from(ay + y)) else {
                    continue;
                };
                if px >= width || py >= height {
                    continue;
                }
                painter.paint(px, py, self.colour);
            }
        }
    }
}

/// Draw `pieces` onto `area` of `buf` through a [`Canvas`].
///
/// A canvas paints its whole area's background, which would wipe out the
/// squares under it. So it draws into a scratch buffer, and only the cells
/// that came out with dots in them are copied across, each keeping the
/// background the board already gave it.
pub fn stamp(buf: &mut Buffer, area: Rect, pieces: &[Piece], dots: Dots) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let (w, h) = (area.width * DOTS.0, area.height * DOTS.1);
    let mut scratch = Buffer::empty(area);
    Canvas::default()
        .marker(dots.marker())
        .x_bounds([0.0, f64::from(w - 1)])
        .y_bounds([0.0, f64::from(h - 1)])
        .paint(|ctx| {
            for piece in pieces {
                ctx.draw(piece);
            }
        })
        .render(area, &mut scratch);

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let drawn = &scratch[(x, y)];
            let Some(new) = dots.decode(drawn.symbol()).filter(|&bits| bits != 0) else {
                continue;
            };
            let fg = drawn.fg;
            // Where a piece passes over another, keep both sets of dots.
            let old = dots.decode(buf[(x, y)].symbol()).unwrap_or(0);
            buf[(x, y)].set_char(dots.encode(new | old)).set_fg(fg);
        }
    }
}

// The silhouettes. Each is a union of simple shapes in a unit box, x across
// and y up from the base, with holes cut back out where a piece has detail.

fn inside(role: Role, x: f64, y: f64) -> bool {
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return false;
    }
    match role {
        Role::Pawn => pawn(x, y),
        Role::Knight => knight(x, y),
        Role::Bishop => bishop(x, y),
        Role::Rook => rook(x, y),
        Role::Queen => queen(x, y),
        Role::King => king(x, y),
    }
}

/// The foot every piece stands on: a broad plinth under a narrower step.
fn base(x: f64, y: f64, half: f64) -> bool {
    rect(x, y, 0.5 - half, 0.0, 0.5 + half, 0.1)
        || rect(x, y, 0.5 - half + 0.07, 0.1, 0.5 + half - 0.07, 0.17)
}

fn pawn(x: f64, y: f64) -> bool {
    // The shortest piece, so it is drawn larger to use the room above it:
    // its head is only a few dots across and needs every one to stay round.
    const SCALE: f64 = 1.12;
    let (x, y) = ((x - 0.5) / SCALE + 0.5, y / SCALE);
    base(x, y, 0.27)
        || taper(x, y, 0.17, 0.46, 0.19, 0.08)
        || ellipse(x, y, 0.5, 0.48, 0.16, 0.045)
        || circle(x, y, 0.5, 0.63, 0.14)
}

fn rook(x: f64, y: f64) -> bool {
    // Three battlements with clear gaps, on a parapet that overhangs a body
    // narrowing slightly as it rises.
    let merlon = |x0: f64| rect(x, y, x0, 0.82, x0 + 0.12, 0.96);
    base(x, y, 0.3)
        || taper(x, y, 0.17, 0.62, 0.22, 0.17)
        || rect(x, y, 0.26, 0.62, 0.74, 0.68)
        || rect(x, y, 0.23, 0.68, 0.77, 0.82)
        || merlon(0.23)
        || merlon(0.44)
        || merlon(0.65)
}

fn bishop(x: f64, y: f64) -> bool {
    let body = base(x, y, 0.28)
        || taper(x, y, 0.17, 0.42, 0.19, 0.09)
        || rect(x, y, 0.33, 0.42, 0.67, 0.47)
        || ellipse(x, y, 0.5, 0.63, 0.16, 0.19)
        || circle(x, y, 0.5, 0.88, 0.06);
    // The mitre's slit, cut diagonally across it.
    let slit = segment(x, y, (0.43, 0.6), (0.6, 0.76), 0.035);
    body && !slit
}

fn knight(x: f64, y: f64) -> bool {
    // Facing left: a long muzzle angled down from the forehead, the jaw cut
    // in sharply under it, an ear standing up at the back of the head, and
    // the mane falling in a curve to the base.
    const HEAD: [(f64, f64); 17] = [
        (0.3, 0.17),
        (0.34, 0.32),
        (0.44, 0.47),
        (0.42, 0.53),
        (0.28, 0.5),
        (0.15, 0.53),
        (0.12, 0.6),
        (0.16, 0.66),
        (0.36, 0.8),
        (0.48, 0.97),
        (0.58, 0.88),
        (0.68, 0.82),
        (0.76, 0.7),
        (0.79, 0.55),
        (0.78, 0.38),
        (0.75, 0.25),
        (0.74, 0.17),
    ];
    let body = base(x, y, 0.3) || polygon(x, y, &HEAD);
    let eye = circle(x, y, 0.42, 0.73, 0.05);
    body && !eye
}

fn queen(x: f64, y: f64) -> bool {
    // A crown sweeping out from the collar into two horns, with a ball held
    // up between them. Spikes a dot or two wide come out square and look
    // like the rook's battlements, so the crown is kept to bold strokes.
    const CROWN: [(f64, f64); 10] = [
        (0.41, 0.52),
        (0.59, 0.52),
        (0.85, 0.83),
        (0.78, 0.91),
        (0.64, 0.76),
        (0.55, 0.8),
        (0.45, 0.8),
        (0.36, 0.76),
        (0.22, 0.91),
        (0.15, 0.83),
    ];
    base(x, y, 0.3)
        || taper(x, y, 0.17, 0.34, 0.21, 0.12)
        || taper(x, y, 0.34, 0.5, 0.12, 0.09)
        || ellipse(x, y, 0.5, 0.51, 0.17, 0.045)
        || polygon(x, y, &CROWN)
        || rect(x, y, 0.45, 0.72, 0.55, 0.84)
        || circle(x, y, 0.5, 0.89, 0.085)
}

fn king(x: f64, y: f64) -> bool {
    base(x, y, 0.3)
        || taper(x, y, 0.17, 0.5, 0.21, 0.11)
        || rect(x, y, 0.33, 0.48, 0.67, 0.54)
        || taper(x, y, 0.54, 0.72, 0.12, 0.2)
        || ellipse(x, y, 0.5, 0.72, 0.2, 0.05)
        || rect(x, y, 0.46, 0.72, 0.54, 1.0)
        || rect(x, y, 0.37, 0.84, 0.63, 0.91)
}

fn rect(x: f64, y: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
}

fn circle(x: f64, y: f64, cx: f64, cy: f64, r: f64) -> bool {
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn ellipse(x: f64, y: f64, cx: f64, cy: f64, rx: f64, ry: f64) -> bool {
    ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0
}

/// A centred column narrowing from `bottom_half` wide at `y0` to `top_half`
/// at `y1`.
fn taper(x: f64, y: f64, y0: f64, y1: f64, bottom_half: f64, top_half: f64) -> bool {
    if !(y0..=y1).contains(&y) {
        return false;
    }
    let t = (y - y0) / (y1 - y0);
    (x - 0.5).abs() <= bottom_half + (top_half - bottom_half) * t
}

/// Within `r` of the line from `a` to `b`.
#[expect(
    clippy::many_single_char_names,
    reason = "the usual names for a point, a line and a radius"
)]
fn segment(x: f64, y: f64, a: (f64, f64), b: (f64, f64), r: f64) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (x - (a.0 + t * dx)).powi(2) + (y - (a.1 + t * dy)).powi(2) <= r * r
}

/// Even-odd point-in-polygon.
fn polygon(x: f64, y: f64, points: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let ((xi, yi), (xj, yj)) = (points[i], points[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}
