//! The evaluation bar: white's share of the chances from white's end, black's
//! from black's, and the score written at the end of whoever is ahead.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::games::chess::analysis::Thinking;
use crate::games::chess::app::App;
use crate::ui::{blend, cells};

const WHITE: Color = Color::Rgb(236, 234, 228);
const BLACK: Color = Color::Rgb(44, 41, 38);
/// Before there is a score, the bar is a ghost of itself.
const GHOST: Color = Color::Rgb(90, 86, 82);

/// Two pixels to a cell, one above the other, so the boundary between white
/// and black can sit halfway down one.
pub(super) fn draw_bar(buf: &mut Buffer, area: Rect, app: &App) {
    if area.is_empty() {
        return;
    }
    let score = match app.thinking() {
        Some(Thinking::Decided(score) | Thinking::Found { score, .. }) => Some(score),
        _ => None,
    };
    let (white, black) = match score {
        Some(_) => (WHITE, BLACK),
        None => (blend(GHOST, WHITE, 0.3), blend(GHOST, BLACK, 0.6)),
    };

    let pixels = usize::from(area.height) * 2;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "a share from 0 to 1 of a column of pixels is a count of them"
    )]
    let lit = ((app.bar_share().clamp(0.0, 1.0) * pixels as f32).round() as usize).min(pixels);
    // White's end is at the bottom unless the board is turned round.
    let is_white = |p: usize| {
        if app.flipped {
            p < lit
        } else {
            p >= pixels - lit
        }
    };
    let colour = |p: usize| if is_white(p) { white } else { black };
    for row in 0..area.height {
        let (top, bottom) = (usize::from(row) * 2, usize::from(row) * 2 + 1);
        for col in 0..area.width {
            buf[(area.x + col, area.y + row)]
                .set_symbol("▀")
                .set_fg(colour(top))
                .set_bg(colour(bottom));
        }
    }

    let Some(score) = score else { return };
    let label = score.label();
    // At the leader's end, or the middle when it is level.
    let at_white = score.white_ahead();
    let white_end_bottom = !app.flipped;
    let row = if at_white == white_end_bottom {
        area.bottom() - 1
    } else {
        area.y
    };
    let text_w = cells(label.chars().count());
    let x = area.x + area.width.saturating_sub(text_w) / 2;
    let (ink, paper) = if at_white {
        (BLACK, WHITE)
    } else {
        (WHITE, BLACK)
    };
    for (i, c) in label.chars().enumerate() {
        let at = Position::new(x + cells(i), row);
        if area.contains(at) {
            buf[at].set_char(c).set_style(
                Style::default()
                    .fg(ink)
                    .bg(paper)
                    .add_modifier(Modifier::BOLD),
            );
        }
    }
}
