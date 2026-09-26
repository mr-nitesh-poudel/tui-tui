//! Drawing the lobby, and the geometry its clicks are tested against.
//!
//! A title in big letters, the menu down the middle with the chosen item
//! boxed, a chessboard running off to the horizon beneath it, and a status
//! bar that says what the chosen item does. On a small screen the scenery
//! goes first, then the big letters, then the room between items, so the
//! menu itself is the last thing to be squeezed.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::{Entry, Field, Item, Lobby, Row};
use crate::games::Kind;
use crate::ui::{
    BRIGHT, CAPTURE, CURSOR, MUTED, SELECTED, blend, cells, centred, keycaps, keycaps_fit,
};

/// Where the lobby's pieces sit. [`LobbyGeometry::rows`] lines up with
/// [`Lobby::rows`], so what is drawn and what is clicked cannot drift apart.
pub struct LobbyGeometry {
    /// The big title, or the one-line one when there is no room for it.
    pub title: Rect,
    pub big_title: bool,
    /// The line under the big title.
    pub tagline: Option<Rect>,
    /// One line per row of the menu, the width of the menu.
    pub rows: Vec<Rect>,
    /// The line saying the friends start here, if there are any.
    pub friends_heading: Option<Rect>,
    /// Rows between menu items: 2 leaves room to box the chosen one.
    pub spacing: u16,
    /// Where a code is typed: the join row, which turns into a text box.
    pub input: Rect,
    /// The status bar: what the chosen item does, or how typing is going.
    pub status: Rect,
    /// The line inside the status bar that the words go on.
    pub hint: Rect,
    /// The chessboard running off to the horizon, when there is room.
    pub floor: Option<Rect>,
    pub footer: Rect,
}

/// The menu's width, and the chosen item's box.
const MENU_W: u16 = 40;
/// The big title: five rows of letters and one of shadow.
const TITLE_H: u16 = 6;
const TITLE: &str = "TUI-TUI";
/// More than this and the floor starts to dominate the screen.
const FLOOR_MAX: u16 = 14;

/// The title's letters, and the shadow they cast.
const INK: Color = Color::Rgb(240, 230, 208);
const SHADOW: Color = Color::Rgb(137, 99, 73);
/// The board's own squares, for the floor.
const FLOOR_LIGHT: Color = Color::Rgb(214, 194, 162);
const FLOOR_DARK: Color = Color::Rgb(137, 99, 73);
const NIGHT: Color = Color::Rgb(0, 0, 0);
/// Unchosen items: readable, but a step back from the chosen one.
const QUIET: Color = Color::Rgb(186, 182, 176);
/// The chosen item, when there is no room to box it.
const HIGHLIGHT: Color = Color::Rgb(58, 54, 50);

/// Something drawn in the menu's column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Row(usize),
    FriendsHeading,
}

fn slots(rows: &[Row]) -> Vec<Slot> {
    let mut out = Vec::with_capacity(rows.len() + 1);
    for (i, row) in rows.iter().enumerate() {
        if *row == Row::Friend(0) {
            out.push(Slot::FriendsHeading);
        }
        out.push(Slot::Row(i));
    }
    out
}

/// How much of the lobby a screen has room for.
#[derive(Clone, Copy)]
struct Plan {
    /// 0 for no title, 1 for one line of it, [`TITLE_H`] for the big one.
    title: u16,
    spacing: u16,
    status: u16,
}

impl Plan {
    /// From the most to the least, each giving up one thing.
    const ALL: [Plan; 5] = [
        Plan {
            title: TITLE_H,
            spacing: 2,
            status: 3,
        },
        Plan {
            title: 1,
            spacing: 2,
            status: 3,
        },
        Plan {
            title: 1,
            spacing: 2,
            status: 1,
        },
        Plan {
            title: 1,
            spacing: 1,
            status: 1,
        },
        Plan {
            title: 0,
            spacing: 1,
            status: 1,
        },
    ];

    /// The title with the line under it and a gap before the menu.
    fn title_h(self) -> u16 {
        match self.title {
            0 => 0,
            TITLE_H => TITLE_H + 2,
            _ => 2,
        }
    }

    /// With room for the chosen item's box above and below it.
    fn menu_h(self, slots: u16) -> u16 {
        if self.spacing == 2 {
            2 * slots + 1
        } else {
            slots
        }
    }

    fn height(self, slots: u16) -> u16 {
        self.title_h() + self.menu_h(slots) + self.status
    }
}

impl LobbyGeometry {
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "the lobby's layout, worked out top to bottom"
    )]
    pub fn new(area: Rect, rows: &[Row]) -> Self {
        let [main, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        let slots = slots(rows);
        let n = cells(slots.len());
        let title_w = title_width() + 1;
        let plan = Plan::ALL
            .into_iter()
            .filter(|p| p.title != TITLE_H || main.width >= title_w + 2)
            .find(|p| p.height(n) <= main.height)
            .unwrap_or(Plan::ALL[Plan::ALL.len() - 1]);

        // What is left goes to the floor, and then either side of the menu.
        let spare = main.height.saturating_sub(plan.height(n));
        let floor_h = if spare >= 6 {
            (spare - 2).min(FLOOR_MAX)
        } else {
            0
        };
        let rest = spare - floor_h;
        let top = main.y + rest / 2;

        let clip = |r: Rect| r.intersection(main);
        // The big title is placed exactly; the one-line one spans the
        // screen, to be centred on it.
        let title = clip(if plan.title == TITLE_H {
            Rect {
                x: main.x + (main.width - title_w) / 2,
                y: top,
                width: title_w,
                height: TITLE_H,
            }
        } else {
            Rect {
                x: main.x,
                y: top,
                width: main.width,
                height: plan.title,
            }
        });
        let tagline = (plan.title == TITLE_H).then(|| {
            clip(Rect {
                x: main.x,
                y: top + TITLE_H,
                width: main.width,
                height: 1,
            })
        });
        let menu_top = top + plan.title_h();
        let menu_w = MENU_W.min(main.width);
        let menu_x = main.x + (main.width - menu_w) / 2;
        let line = |k: u16| {
            let y = if plan.spacing == 2 {
                menu_top + 1 + 2 * k
            } else {
                menu_top + k
            };
            clip(Rect {
                x: menu_x,
                y,
                width: menu_w,
                height: 1,
            })
        };
        let mut placed = Vec::with_capacity(rows.len());
        let mut friends_heading = None;
        for (k, slot) in slots.iter().enumerate() {
            let at = line(cells(k));
            match slot {
                Slot::Row(_) => placed.push(at),
                Slot::FriendsHeading => friends_heading = Some(at),
            }
        }
        let input = rows
            .iter()
            .position(|r| *r == Row::Item(Item::Join))
            .and_then(|i| placed.get(i).copied())
            .unwrap_or_default();

        let status = clip(Rect {
            x: main.x,
            y: main.bottom().saturating_sub(plan.status),
            width: main.width,
            height: plan.status,
        });
        let hint = if plan.status == 3 {
            clip(Rect {
                x: status.x + 2,
                y: status.y + 1,
                width: status.width.saturating_sub(4),
                height: 1,
            })
        } else {
            status
        };
        let floor = (floor_h > 0).then(|| {
            clip(Rect {
                x: main.x,
                y: status.y.saturating_sub(floor_h),
                width: main.width,
                height: floor_h,
            })
        });

        Self {
            title,
            big_title: plan.title == TITLE_H,
            tagline,
            rows: placed,
            friends_heading,
            spacing: plan.spacing,
            input,
            status,
            hint,
            floor,
            footer,
        }
    }

    /// The row under a screen position, if any.
    #[must_use]
    pub fn row_at(&self, x: u16, y: u16) -> Option<usize> {
        self.rows.iter().position(|r| r.contains(Position { x, y }))
    }
}

pub fn draw_lobby(f: &mut Frame, lobby: &Lobby) {
    let rows = lobby.rows();
    let g = LobbyGeometry::new(f.area(), &rows);
    let muted = Style::default().fg(MUTED);

    // Scenery first, so everything else is drawn over it.
    if let Some(floor) = g.floor {
        draw_floor(f.buffer_mut(), floor);
    }
    let screen = f.area();
    draw_stars(f.buffer_mut(), screen, &g);

    draw_title(f, &g);

    if let Some(area) = g.friends_heading {
        f.render_widget(
            Paragraph::new(Line::styled("─── friends ───", muted).centered()),
            area,
        );
    }

    for (i, (&row, &area)) in rows.iter().zip(&g.rows).enumerate() {
        let chosen = i == lobby.selected;
        let editing = chosen && lobby.editing.is_some();
        if chosen && g.spacing == 2 {
            let border = if editing { CURSOR } else { QUIET };
            let boxed = Rect {
                y: area.y.saturating_sub(1),
                height: 3,
                ..area
            }
            .intersection(f.area());
            f.render_widget(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border)),
                boxed,
            );
        } else if chosen {
            f.render_widget(Block::default().style(Style::default().bg(HIGHLIGHT)), area);
        }
        let inner = Rect {
            x: area.x + 2,
            width: area.width.saturating_sub(4),
            ..area
        };
        if editing {
            let (line, cursor) = edit_line(lobby);
            f.render_widget(Paragraph::new(line), inner);
            set_cursor(f, inner, cursor);
        } else {
            f.render_widget(Paragraph::new(row_line(lobby, row, chosen)), inner);
        }
    }

    draw_status(f, &g, lobby, &rows);
    draw_footer(f, g.footer, lobby, &rows);

    if let Some(invite) = &lobby.invite {
        let area = centred(f.area(), 44, 6);
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(CURSOR))
            .title(Line::from(" invite ").centered());
        let lines = vec![
            Line::raw(""),
            Line::styled(
                format!(
                    "{} wants to play {}",
                    invite.name,
                    invite.game.name().to_lowercase()
                ),
                Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            )
            .centered(),
            Line::raw(""),
            Line::from(keycaps(&[("y", "accept"), ("n", "decline")])).centered(),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    }
}

/// What a row says, centred in the menu.
fn row_line(lobby: &Lobby, row: Row, chosen: bool) -> Line<'static> {
    let muted = Style::default().fg(MUTED);
    let label = if chosen {
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(QUIET)
    };
    let spans = match row {
        Row::Item(Item::Game) => {
            // The arrows say it can be changed, once there is a choice.
            let arrows = if Kind::ALL.len() > 1 && chosen {
                Style::default().fg(CURSOR)
            } else {
                muted
            };
            vec![
                Span::styled("Game  ", muted),
                Span::styled("‹ ", arrows),
                Span::styled(lobby.game().name(), label),
                Span::styled(" ›", arrows),
            ]
        }
        Row::Item(Item::Join) => vec![Span::styled("Join with a code", label)],
        Row::Item(Item::Name) => {
            let mut spans = vec![
                Span::styled("Name  ", muted),
                Span::styled(lobby.name.clone(), label),
            ];
            if lobby.guest {
                spans.push(Span::styled(" (guest)", muted));
            }
            spans
        }
        Row::Item(Item::Local) if lobby.game().alone() => vec![Span::styled("Play alone", label)],
        Row::Item(item) => vec![Span::styled(item.label(), label)],
        Row::Friend(n) => {
            let Some(friend) = lobby.friends.get(n) else {
                return Line::raw("");
            };
            let games = match friend.games {
                1 => "1 game".to_string(),
                n => format!("{n} games"),
            };
            let name: String = friend.name.chars().take(16).collect();
            vec![
                Span::styled(name, label),
                Span::styled(format!("  {games} · {}", ago(friend.last_played)), muted),
            ]
        }
    };
    Line::from(spans).centered()
}

/// The row being typed into, and how far along it the cursor sits.
fn edit_line(lobby: &Lobby) -> (Line<'static>, usize) {
    let muted = Style::default().fg(MUTED);
    let prompt = Span::styled("› ", Style::default().fg(CURSOR));
    let typed = Style::default().fg(BRIGHT);
    match lobby.editing {
        Some(Field::Name) => {
            let text = lobby.name_input.clone();
            let at = 2 + text.chars().count();
            (Line::from(vec![prompt, Span::styled(text, typed)]), at)
        }
        _ if lobby.input.is_empty() => (
            Line::from(vec![prompt, Span::styled("42-tiger-marble-ocean", muted)]),
            2,
        ),
        _ => {
            // The rest of the word Tab would fill in, greyed out after the
            // cursor.
            let ghost = lobby.completion().map_or("", |word| {
                let typed = lobby.input.rsplit('-').next().map_or(0, str::len);
                &word[typed..]
            });
            let at = 2 + lobby.input.len();
            (
                Line::from(vec![
                    prompt,
                    Span::styled(lobby.input.clone(), typed),
                    Span::styled(ghost, muted),
                ]),
                at,
            )
        }
    }
}

/// What the chosen row does, or how typing is going, or whatever the lobby
/// has to say.
fn status_line(lobby: &Lobby, rows: &[Row]) -> Line<'static> {
    let muted = Style::default().fg(QUIET);
    if let Some(i) = lobby.forgetting {
        let name = lobby.friends.get(i).map_or("them", |f| f.name.as_str());
        return Line::styled(
            format!("forget {name}? y to confirm, anything else keeps them"),
            Style::default().fg(CAPTURE),
        );
    }
    match lobby.editing {
        Some(Field::Name) => {
            return Line::styled("what friends will see you as — enter saves", muted);
        }
        Some(Field::Code) => {
            return match lobby.entry() {
                Entry::Empty => Line::styled("type or paste the code you were sent", muted),
                Entry::Typing if lobby.completion().is_some() => {
                    Line::styled("tab finishes the word", muted)
                }
                Entry::Typing => Line::styled("a number and three words", muted),
                Entry::Ready(_) => Line::styled(
                    "that's a code — enter to join",
                    Style::default().fg(SELECTED),
                ),
                Entry::Bad(why) => Line::styled(why, Style::default().fg(CAPTURE)),
            };
        }
        None => {}
    }
    if let Some(notice) = &lobby.notice {
        return Line::styled(notice.clone(), Style::default().fg(CURSOR));
    }
    let game = lobby.game().name().to_lowercase();
    let text = match rows.get(lobby.selected) {
        Some(Row::Item(Item::Game)) if Kind::ALL.len() > 1 => {
            "what hosting, joining or a challenge will play — ←/→ to change".into()
        }
        Some(Row::Item(Item::Game)) => format!("{game} is the game on the table"),
        Some(Row::Item(Item::Host)) => {
            "get a code to send your opponent, then wait for them here".into()
        }
        Some(Row::Item(Item::Join)) => {
            "type in the code your opponent sent — or just start typing it".into()
        }
        Some(Row::Item(Item::Local)) if lobby.game().alone() => {
            format!("a game of {game} on your own")
        }
        Some(Row::Item(Item::Local)) => "two players taking turns at this keyboard".into(),
        Some(Row::Item(Item::Name)) if lobby.guest => {
            "a guest: another copy of tuitui has your profile open".into()
        }
        Some(Row::Item(Item::Name)) if lobby.friends.is_empty() => {
            "what friends see — anyone you play turns up as a friend".into()
        }
        Some(Row::Item(Item::Name)) => "what friends see".into(),
        Some(Row::Item(Item::Quit)) => "see you next time".into(),
        Some(Row::Friend(n)) => {
            let name = lobby.friends.get(*n).map_or("them", |f| f.name.as_str());
            format!("challenge {name} to a game of {game}")
        }
        None => String::new(),
    };
    Line::styled(text, muted)
}

fn draw_status(f: &mut Frame, g: &LobbyGeometry, lobby: &Lobby, rows: &[Row]) {
    let line = status_line(lobby, rows);
    if g.status.height >= 3 {
        f.render_widget(Clear, g.status);
        f.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED)),
            g.status,
        );
        f.render_widget(Paragraph::new(line), g.hint);
    } else {
        f.render_widget(Paragraph::new(line.centered()), g.hint);
    }
}

fn draw_footer(f: &mut Frame, area: Rect, lobby: &Lobby, rows: &[Row]) {
    let on_friend = matches!(rows.get(lobby.selected), Some(Row::Friend(_)));
    let keys: &[(&str, &str)] = if lobby.invite.is_some() {
        &[("y", "accept"), ("n", "decline")]
    } else if lobby.editing == Some(Field::Code) {
        &[
            ("enter", "join"),
            ("tab", "complete"),
            ("ctrl-u", "clear"),
            ("esc", "back"),
        ]
    } else if lobby.editing == Some(Field::Name) {
        &[("enter", "save"), ("esc", "cancel")]
    } else if lobby.on_game() && Kind::ALL.len() > 1 {
        &[("←/→", "change game"), ("↑/↓", "select"), ("q", "quit")]
    } else if on_friend {
        &[
            ("enter", "challenge"),
            ("x", "forget"),
            ("↑/↓", "select"),
            ("q", "quit"),
        ]
    } else {
        &[("↑/↓", "select"), ("enter", "confirm"), ("q", "quit")]
    };

    // As many keys as fit, the most useful first.
    let who = if lobby.name.is_empty() {
        None
    } else {
        Some(format!("playing as {} ", lobby.name))
    };
    let who_w = who.as_ref().map_or(0, |w| cells(w.chars().count()) + 2);
    let mut left = vec![Span::raw(" ")];
    left.extend(keycaps_fit(keys, area.width.saturating_sub(who_w + 1)));
    let used = cells(Line::from(left.clone()).width());
    f.render_widget(Paragraph::new(Line::from(left)), area);
    if let Some(who) = who
        && used + who_w <= area.width
    {
        f.render_widget(
            Paragraph::new(Line::styled(who, Style::default().fg(MUTED)).right_aligned()),
            area,
        );
    }
}

/// The title's letters on a grid of square pixels, two to a cell stacked
/// one above the other: `#` is ink.
fn glyph(c: char) -> &'static [&'static str] {
    match c {
        'T' => &[
            "########", "########", "...##...", "...##...", "...##...", "...##...", "...##...",
            "...##...", "...##...", "...##...",
        ],
        'U' => &[
            "##....##", "##....##", "##....##", "##....##", "##....##", "##....##", "##....##",
            "##....##", ".######.", ".######.",
        ],
        'I' => &[
            "######", "######", "..##..", "..##..", "..##..", "..##..", "..##..", "..##..",
            "######", "######",
        ],
        '-' => &[
            ".....", ".....", ".....", ".....", "#####", "#####", ".....", ".....", ".....",
            ".....",
        ],
        _ => &[],
    }
}

/// Pixels between letters.
const LETTER_GAP: u16 = 2;

fn title_width() -> u16 {
    let letters: u16 = TITLE
        .chars()
        .map(|c| glyph(c).first().map_or(0, |r| cells(r.len())))
        .sum();
    letters + LETTER_GAP * (cells(TITLE.chars().count()) - 1)
}

/// Every inked pixel of the title, from its top left.
fn title_pixels() -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    let mut x = 0;
    for c in TITLE.chars() {
        let rows = glyph(c);
        for (y, row) in rows.iter().enumerate() {
            for (dx, p) in row.chars().enumerate() {
                if p == '#' {
                    out.push((x + cells(dx), cells(y)));
                }
            }
        }
        x += rows.first().map_or(0, |r| cells(r.len())) + LETTER_GAP;
    }
    out
}

#[expect(
    clippy::many_single_char_names,
    reason = "screen geometry, named as it is everywhere else here"
)]
fn draw_title(f: &mut Frame, g: &LobbyGeometry) {
    let version = format!("terminal games for two · v{}", env!("CARGO_PKG_VERSION"));
    if !g.big_title {
        let line = Line::from(vec![
            Span::styled(
                "tui-tui",
                Style::default().fg(INK).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(MUTED),
            ),
        ]);
        f.render_widget(Paragraph::new(line.centered()), g.title);
        return;
    }

    // Paint the shadow a pixel down and to the right, then the ink over it.
    let (w, h) = (usize::from(g.title.width), usize::from(g.title.height) * 2);
    let mut pixels: Vec<Option<Color>> = vec![None; w * h];
    let inked = title_pixels();
    for (offset, colour) in [(1, SHADOW), (0, INK)] {
        for &(x, y) in &inked {
            let (x, y) = (usize::from(x + offset), usize::from(y + offset));
            if x < w && y < h {
                pixels[y * w + x] = Some(colour);
            }
        }
    }
    let buf = f.buffer_mut();
    for row in 0..g.title.height {
        for col in 0..g.title.width {
            let (x, y) = (usize::from(col), usize::from(row) * 2);
            let cell = &mut buf[(g.title.x + col, g.title.y + row)];
            match (pixels[y * w + x], pixels[(y + 1) * w + x]) {
                (None, None) => {}
                (Some(top), Some(bottom)) if top == bottom => {
                    cell.set_symbol("█").set_fg(top);
                }
                (Some(top), bottom) => {
                    cell.set_symbol("▀").set_fg(top);
                    if let Some(bottom) = bottom {
                        cell.set_bg(bottom);
                    }
                }
                (None, Some(bottom)) => {
                    cell.set_symbol("▄").set_fg(bottom);
                }
            }
        }
    }
    if let Some(area) = g.tagline {
        f.render_widget(
            Paragraph::new(Line::styled(version, Style::default().fg(MUTED)).centered()),
            area,
        );
    }
}

/// A chessboard seen from just above it, running off to a dark horizon. Each
/// cell is two pixels stacked, the top in the foreground of a `▀` and the
/// bottom in its background.
fn draw_floor(buf: &mut Buffer, area: Rect) {
    const SAMPLES: u16 = 4;
    let depth = f64::from(area.height * 2);
    let centre = f64::from(area.width) / 2.0;
    // Squares at the front are this many cells across.
    let near = 12.0;
    let camera = depth / near;
    let focal = 2.0 * depth;
    // How much of a pixel is light square, sampled on a grid within it so
    // the far squares blur together rather than shimmer.
    let light_share = |px: u16, py: u16| {
        let mut light = 0u16;
        for sy in 0..SAMPLES {
            for sx in 0..SAMPLES {
                // How far below the horizon, which is how close to us.
                let d = f64::from(py) + (f64::from(sy) + 0.5) / f64::from(SAMPLES);
                let x = f64::from(px) + (f64::from(sx) + 0.5) / f64::from(SAMPLES);
                let wx = (x - centre) * camera / d;
                let wz = camera * focal / d;
                if (wx.floor() + wz.floor()).rem_euclid(2.0) < 1.0 {
                    light += 1;
                }
            }
        }
        f32::from(light) / f32::from(SAMPLES * SAMPLES)
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "fractions from 0 to 1, which need no more than f32"
    )]
    let pixel = |px: u16, py: u16| {
        let square = blend(FLOOR_DARK, FLOOR_LIGHT, light_share(px, py));
        // Fading into the dark at the horizon and towards either side, and
        // never as bright as the board itself.
        let near = ((f64::from(py) + 0.5) / depth) as f32;
        let side = ((f64::from(px) + 0.5 - centre) / centre).abs() as f32;
        let alpha = near.powf(1.6) * 0.55 * (1.0 - 0.7 * side.powi(2));
        blend(NIGHT, square, alpha)
    };
    for row in 0..area.height {
        for col in 0..area.width {
            let at = Position::new(area.x + col, area.y + row);
            buf[at]
                .set_symbol("▀")
                .set_fg(pixel(col, 2 * row))
                .set_bg(pixel(col, 2 * row + 1));
        }
    }
}

/// A scattering of stars in the sky, clear of the title, the menu and the
/// floor. Where they fall depends only on the screen position, so they stay
/// put from one frame to the next.
fn draw_stars(buf: &mut Buffer, area: Rect, g: &LobbyGeometry) {
    let sky_bottom = g.floor.map_or(g.status.y, |f| f.y);
    let menu = g
        .rows
        .iter()
        .chain(&g.friends_heading)
        .fold(g.title, |acc, r| acc.union(*r));
    // A margin round what is written, so no star crowds it.
    let keep_clear = Rect {
        x: menu.x.saturating_sub(4),
        y: menu.y.saturating_sub(1),
        width: menu.width + 8,
        height: menu.height + 3,
    };
    for y in area.y..sky_bottom.min(area.bottom()) {
        for x in area.x..area.right() {
            let at = Position::new(x, y);
            if keep_clear.contains(at) {
                continue;
            }
            let h = hash(x, y);
            let (symbol, colour) = match h % 97 {
                0 => ("✦", CURSOR),
                1 | 2 => ("·", QUIET),
                3 => ("·", MUTED),
                _ => continue,
            };
            buf[at].set_symbol(symbol).set_fg(colour);
        }
    }
}

fn hash(x: u16, y: u16) -> u32 {
    let mut h = u32::from(x).wrapping_mul(0x9E37_79B1) ^ u32::from(y).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

fn set_cursor(f: &mut Frame, line: Rect, offset: usize) {
    if line.width == 0 || line.height == 0 {
        return;
    }
    let x = line.x.saturating_add(cells(offset));
    f.set_cursor_position(Position {
        x: x.min(line.right().saturating_sub(1)),
        y: line.y,
    });
}

/// How long ago a Unix time was, in the loosest terms that still help.
fn ago(then: u64) -> String {
    const HOUR: u64 = 60 * 60;
    const DAY: u64 = 24 * HOUR;
    const TWO_DAYS: u64 = 2 * DAY;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let s = now.saturating_sub(then);
    match s {
        0..60 => "just now".into(),
        60..HOUR => format!("{}m ago", s / 60),
        HOUR..DAY => format!("{}h ago", s / HOUR),
        DAY..TWO_DAYS => "yesterday".into(),
        _ if s < 14 * DAY => format!("{}d ago", s / DAY),
        _ => format!("{}w ago", s / (7 * DAY)),
    }
}
