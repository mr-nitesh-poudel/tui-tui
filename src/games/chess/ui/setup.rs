//! The choices before a game against the bot: how strong it plays, and which
//! side you take, in a box over the board.

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::SetupGeometry;
use super::panels::{HIGHLIGHT, QUIET};
use crate::games::chess::app::App;
use crate::games::chess::bot::Setting;
use crate::ui::{BRIGHT, CURSOR, MUTED, SELECTED};

/// The words on the button that starts the game.
const DARK_INK: Color = Color::Rgb(22, 30, 18);

/// Draws the choices while they are being made, and nothing once the game
/// has started.
pub(super) fn draw_setup(f: &mut Frame, g: &SetupGeometry, app: &App) {
    let (Some(versus), Some(on)) = (&app.versus, app.choosing()) else {
        return;
    };
    f.render_widget(Clear, g.area);
    f.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(CURSOR))
            .title(Line::styled(" play a bot ", Style::default().fg(BRIGHT).bold()).centered()),
        g.area,
    );

    for (i, &setting) in Setting::ALL.iter().enumerate() {
        let chosen = setting == on;
        if chosen {
            f.render_widget(
                Block::default().style(Style::default().bg(HIGHLIGHT)),
                g.lines[i],
            );
        }
        let (label, arrow) = if chosen {
            (
                Style::default().fg(BRIGHT).bold(),
                Style::default().fg(CURSOR),
            )
        } else {
            (Style::default().fg(QUIET), Style::default().fg(MUTED))
        };
        let text = |s: String, style: Style| Paragraph::new(Line::styled(s, style).centered());
        f.render_widget(
            Paragraph::new(Line::styled(setting.label(), label)),
            g.labels[i],
        );
        f.render_widget(text("◂".into(), arrow), g.less[i]);
        f.render_widget(
            text(versus.value(setting), Style::default().fg(BRIGHT).bold()),
            g.values[i],
        );
        f.render_widget(text("▸".into(), arrow), g.more[i]);
    }

    // While an engine is fetched to play, that is what the game waits on.
    if app.download.is_some() {
        f.render_widget(
            Paragraph::new(
                Line::styled("fetching the engine…", Style::default().fg(MUTED)).centered(),
            ),
            g.foot,
        );
    } else {
        let button = Style::default().bg(SELECTED).fg(DARK_INK).bold();
        f.render_widget(
            Paragraph::new(Line::styled("start", button).centered()).style(button),
            g.start,
        );
    }
}
