//! What every screen draws with: the palette, and a few small helpers.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// The cursor, and anything else the player's eye should land on.
pub const CURSOR: Color = Color::Rgb(246, 205, 82);
/// Something picked, or ready to go.
pub const SELECTED: Color = Color::Rgb(124, 176, 95);
/// A warning, or a question that needs an answer.
pub const CAPTURE: Color = Color::Rgb(204, 96, 78);
/// Labels, hints and borders.
pub const MUTED: Color = Color::Rgb(128, 128, 128);

/// Mixes `over` into `base` at `alpha`. Terminals have no alpha channel, so
/// the blend happens here and is handed over as one solid colour.
#[must_use]
pub fn blend(base: Color, over: Color, alpha: f32) -> Color {
    let (Color::Rgb(br, bg, bb), Color::Rgb(or, og, ob)) = (base, over) else {
        return over;
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a blend of two bytes is itself a byte, for alpha from 0 to 1"
    )]
    let mix = |b: u8, o: u8| (f32::from(b) * (1.0 - alpha) + f32::from(o) * alpha).round() as u8;
    Color::Rgb(mix(br, or), mix(bg, og), mix(bb, ob))
}

/// `n` as a number of terminal cells. Nothing on screen comes near
/// `u16::MAX` of them, but anything that did is clamped rather than wrapped.
#[must_use]
pub fn cells(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// A `w` by `h` rectangle in the middle of `area`, shrunk to fit.
#[must_use]
pub fn centred(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// Text that should stand out from the ordinary.
pub const BRIGHT: Color = Color::Rgb(250, 248, 244);
/// The little key a hint is printed on.
pub const KEYCAP: Color = Color::Rgb(72, 70, 68);

/// Keys as little caps, each followed by what it does.
#[must_use]
pub fn keycaps(keys: &[(&str, &str)]) -> Vec<Span<'static>> {
    let cap = Style::default().bg(KEYCAP).fg(BRIGHT);
    let muted = Style::default().fg(MUTED);
    let mut spans = Vec::new();
    for (i, (key, what)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!(" {key} "), cap));
        spans.push(Span::styled(format!(" {what}"), muted));
    }
    spans
}

/// As many of `keys` as fit in `width`, in order. Keys go from the end
/// first, except the last, which is always the way out and always kept.
#[must_use]
pub fn keycaps_fit(keys: &[(&str, &str)], width: u16) -> Vec<Span<'static>> {
    let width = usize::from(width);
    let Some((last, rest)) = keys.split_last() else {
        return Vec::new();
    };
    for shown in (0..=rest.len()).rev() {
        let mut kept = rest[..shown].to_vec();
        kept.push(*last);
        let spans = keycaps(&kept);
        if Line::from(spans.clone()).width() <= width || shown == 0 {
            return spans;
        }
    }
    Vec::new()
}
