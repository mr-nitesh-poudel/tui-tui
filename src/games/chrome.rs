//! What every game's screen shares: where the connection stands, and the
//! footer with the table's own questions in it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Conn, Ctx};
use crate::clipboard::Copied;
use crate::ui::{CAPTURE, CURSOR, MUTED, SELECTED, cells, keycaps, keycaps_fit};

/// A few lines on the connection while there is no opponent to show: the
/// code to share while hosting, who we are waiting on, or what went wrong.
/// Nothing in hot-seat or once playing, where the opponent speaks for it.
pub fn connection_lines(ctx: &Ctx) -> Vec<Line<'static>> {
    let muted = Style::default().fg(MUTED);
    let mut lines = Vec::new();
    match &ctx.conn {
        Conn::Local | Conn::Playing => {}
        Conn::Publishing => lines.push(Line::styled("publishing a code…", muted)),
        Conn::Waiting => {
            lines.push(Line::styled(
                ctx.share.clone().unwrap_or_default(),
                Style::default().fg(CURSOR).add_modifier(Modifier::BOLD),
            ));
            match ctx.copied {
                Some(Copied::Clipboard) => {
                    lines.push(Line::styled("copied ✓", Style::default().fg(SELECTED)));
                }
                // Nothing reports back whether the terminal did it, so say
                // what to do if it did not.
                Some(Copied::Terminal) => {
                    lines.push(Line::styled(
                        "copied via the terminal",
                        Style::default().fg(SELECTED),
                    ));
                    if ctx.mouse {
                        lines.push(Line::styled("(no? m, then select it)", muted));
                    }
                }
                None => lines.push(Line::from(keycaps(&[("c", "copy")]))),
            }
        }
        Conn::LookingUp => lines.push(Line::styled("looking up the code…", muted)),
        Conn::Dialling => lines.push(Line::styled("connecting…", muted)),
        Conn::Inviting(name) => {
            lines.push(Line::styled(
                format!("waiting for {name} to accept…"),
                muted,
            ));
        }
        Conn::Lost(why) => lines.push(Line::styled(why.clone(), Style::default().fg(CAPTURE))),
    }
    lines
}

/// A dot for how the connection is: green while playing, amber while it is
/// being made, red once it is lost. Nothing in hot-seat.
pub fn connection_dot(ctx: &Ctx) -> Option<Span<'static>> {
    let colour = match ctx.conn {
        Conn::Local => return None,
        Conn::Playing => SELECTED,
        Conn::Lost(_) => CAPTURE,
        _ => CURSOR,
    };
    Some(Span::styled("●", Style::default().fg(colour)))
}

/// A question the footer is asking, and the keys that answer it.
pub type Question<'a> = (&'a str, &'a [(&'a str, &'a str)]);

/// The bottom line of the screen. The table's own question comes first, then
/// the chat's keys while the player is typing, then the game's question, and
/// with none of those the game's keys. Keys that do not fit are dropped from
/// the end, but the last, the way out, always stays.
pub fn footer(
    f: &mut Frame,
    area: Rect,
    ctx: &Ctx,
    question: Option<Question>,
    keys: &[(&str, &str)],
) {
    let chat_keys: &[(&str, &str)] = if ctx.is_networked() {
        &[
            ("enter", "send"),
            ("↑/↓", "scroll"),
            ("esc", "back to the game"),
        ]
    } else {
        &[("↑/↓", "scroll"), ("esc", "back to the game")]
    };
    let (asked, keys) = if ctx.confirm_leave {
        (
            Some("leave this game?"),
            &[("y", "leave"), ("n", "stay")][..],
        )
    } else if ctx.chat.focused {
        (None, chat_keys)
    } else if let Some((text, answers)) = question {
        (Some(text), answers)
    } else {
        (None, keys)
    };

    let mut spans = Vec::new();
    if let Some(text) = asked {
        // A question waiting on an answer stands out from the usual keys.
        let asking = Style::default().fg(CAPTURE).add_modifier(Modifier::BOLD);
        spans.push(Span::styled(format!("{text}   "), asking));
    }
    let used = cells(Line::from(spans.clone()).width());
    spans.extend(keycaps_fit(keys, area.width.saturating_sub(used)));
    f.render_widget(Paragraph::new(Line::from(spans).centered()), area);
}
