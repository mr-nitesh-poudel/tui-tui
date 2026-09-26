//! Drawing the lobby, and the geometry its clicks are tested against.
//!
//! A header with who is playing, a shelf of game cards, and under it two
//! columns: what can be done with the game picked, and the friends it can be
//! played with. Beneath them, a box to type a code into and join; the status
//! bar at the bottom says what the chosen row does, or how the code is
//! looking. On a small screen the thumbnails go first, then the cards shrink
//! to a line of tabs, then the friends move under the actions, the status bar
//! and then the code box lose their borders, so the rows themselves are the
//! last thing to be squeezed. Stars fill whatever sky is left.

use ratatui::layout::{Position, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::{Entry, Field, Item, Lobby, Row};
use crate::games::Kind;
use crate::session::name::NAME_MAX;
use crate::ui::{BRIGHT, CAPTURE, CURSOR, MUTED, SELECTED, cells, centred, keycaps, keycaps_fit};

/// How the games are shown along the top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    /// A boxed card for each, with a picture of the game if `thumbs`.
    Cards { thumbs: bool },
    /// A line of names.
    Tabs,
}

impl Shelf {
    fn height(self) -> u16 {
        match self {
            // The box, the name and the blurb, and the picture.
            Shelf::Cards { thumbs: true } => THUMB_H + 4,
            Shelf::Cards { thumbs: false } => 4,
            Shelf::Tabs => 1,
        }
    }
}

/// Where the lobby's pieces sit. [`LobbyGeometry::rows`] lines up with
/// [`Lobby::rows`] and [`LobbyGeometry::cards`] with [`Kind::ALL`], so what is
/// drawn and what is clicked cannot drift apart.
pub struct LobbyGeometry {
    /// The top line: the program's name, and who is playing.
    pub header: Rect,
    /// Who is playing, in the header; a click renames.
    pub name: Rect,
    pub shelf: Shelf,
    /// One per game: a card, or a tab.
    pub cards: Vec<Rect>,
    /// Above the actions, and above the friends.
    pub actions_heading: Rect,
    pub friends_heading: Rect,
    /// Where the friends would be, when there are none.
    pub friends_note: Rect,
    /// One per row of the lobby: a line the width of its column, or for
    /// [`Row::Code`] the whole code box.
    pub rows: Vec<Rect>,
    /// The code box, boxed or a bare line.
    pub code_box: Rect,
    /// The line in the code box that a code is typed on.
    pub input: Rect,
    /// The status bar: what the chosen row does, or how the code is looking.
    pub status: Rect,
    /// The line inside the status bar that the words go on.
    pub hint: Rect,
    pub footer: Rect,
}

/// The cards and columns never spread wider than this.
const CONTENT_W: u16 = 72;
/// At most, and at least before the cards turn into tabs.
const CARD_W: u16 = 32;
const CARD_MIN_W: u16 = 18;
const CARD_GAP: u16 = 2;
/// Rows of picture on a card.
const THUMB_H: u16 = 4;
/// From this wide, the friends sit beside the actions rather than under.
const TWO_COLUMNS: u16 = 56;
const GUTTER: u16 = 4;

/// The program's name, in the header.
const INK: Color = Color::Rgb(240, 230, 208);
/// Unchosen rows: readable, but a step back from the chosen one.
const QUIET: Color = Color::Rgb(186, 182, 176);
/// Behind the chosen row, and the chosen tab.
const HIGHLIGHT: Color = Color::Rgb(58, 54, 50);

/// How much of the lobby a screen has room for.
#[derive(Clone, Copy)]
struct Plan {
    shelf: Shelf,
    /// 3 for a boxed status bar, 1 for a bare line.
    status: u16,
    /// The same, for the code box.
    field: u16,
    /// Blank lines between the parts.
    gap: u16,
}

impl Plan {
    /// From the most to the least, each giving up one thing.
    const ALL: [Plan; 6] = [
        Plan {
            shelf: Shelf::Cards { thumbs: true },
            status: 3,
            field: 3,
            gap: 1,
        },
        Plan {
            shelf: Shelf::Cards { thumbs: false },
            status: 3,
            field: 3,
            gap: 1,
        },
        Plan {
            shelf: Shelf::Tabs,
            status: 3,
            field: 3,
            gap: 1,
        },
        Plan {
            shelf: Shelf::Tabs,
            status: 1,
            field: 3,
            gap: 1,
        },
        Plan {
            shelf: Shelf::Tabs,
            status: 1,
            field: 1,
            gap: 1,
        },
        Plan {
            shelf: Shelf::Tabs,
            status: 1,
            field: 1,
            gap: 0,
        },
    ];

    /// What goes between the header and the status bar: shelf, body and
    /// code box, with the gaps between.
    fn content(self, body: u16) -> u16 {
        self.shelf.height() + self.gap + body + self.gap + self.field
    }

    /// Everything, with a gap either side of the content.
    fn height(self, body: u16) -> u16 {
        1 + self.gap + self.content(body) + self.gap + self.status + 1
    }
}

/// What the header says on the right: who is playing.
fn who(lobby: &Lobby) -> String {
    let guest = if lobby.guest { " (guest)" } else { "" };
    format!("playing as {}{guest}", lobby.name)
}

impl LobbyGeometry {
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "the lobby's layout, worked out top to bottom"
    )]
    pub fn new(area: Rect, lobby: &Lobby) -> Self {
        let rows = lobby.rows();
        let clip = |r: Rect| r.intersection(area);
        let line = |x: u16, y: u16, width: u16| clip(Rect::new(x, y, width, 1));

        let content_w = area.width.saturating_sub(4).min(CONTENT_W);
        let content_x = area.x + (area.width - content_w) / 2;
        let two_columns = content_w >= TWO_COLUMNS;

        let games = cells(Kind::ALL.len());
        let card_w = (content_w.saturating_sub(CARD_GAP * (games - 1)) / games).min(CARD_W);
        let friends = cells(rows.iter().filter(|r| matches!(r, Row::Friend(_))).count());
        let actions_h = 1 + cells(Item::ALL.len());
        let friends_h = 1 + friends.max(1);

        // The actions and friends, side by side or one above the other.
        let body_h = |gap: u16| {
            if two_columns {
                actions_h.max(friends_h)
            } else {
                actions_h + gap + friends_h
            }
        };
        let fits = |p: &Plan| {
            let wide_enough = p.shelf == Shelf::Tabs || card_w >= CARD_MIN_W;
            wide_enough && p.height(body_h(p.gap)) <= area.height
        };
        let plan = Plan::ALL
            .into_iter()
            .find(fits)
            .unwrap_or(Plan::ALL[Plan::ALL.len() - 1]);
        let gap = plan.gap;
        let body_h = body_h(gap);

        // Header and footer hug the edges, the status bar sits on the
        // footer, and the shelf and body share what is between, centred.
        let header = line(area.x, area.y, area.width);
        let who_w = cells(who(lobby).chars().count()) + 1;
        let name = line(
            area.right().saturating_sub(who_w),
            area.y,
            who_w.min(area.width),
        );
        let footer = line(area.x, area.bottom().saturating_sub(1), area.width);
        let status = clip(Rect {
            x: area.x,
            y: footer.y.saturating_sub(plan.status),
            width: area.width,
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
        let top = area.y.saturating_add(1 + gap);
        let room = status.y.saturating_sub(gap).saturating_sub(top);
        let content_h = plan.content(body_h);
        let shelf_y = top.saturating_add(room.saturating_sub(content_h) / 2);

        let cards: Vec<Rect> = match plan.shelf {
            Shelf::Cards { .. } => {
                let across = card_w * games + CARD_GAP * (games - 1);
                let x0 = content_x + (content_w - across.min(content_w)) / 2;
                (0..games)
                    .map(|i| {
                        clip(Rect {
                            x: x0.saturating_add(i * (card_w + CARD_GAP)),
                            y: shelf_y,
                            width: card_w,
                            height: plan.shelf.height(),
                        })
                    })
                    .collect()
            }
            Shelf::Tabs => {
                // Each name with a space either side, and a bar between.
                let widths: Vec<u16> = Kind::ALL
                    .iter()
                    .map(|k| cells(k.name().chars().count()) + 2)
                    .collect();
                let across = widths.iter().sum::<u16>() + (games - 1);
                let mut x = area.x + area.width.saturating_sub(across) / 2;
                widths
                    .iter()
                    .map(|&w| {
                        let tab = line(x, shelf_y, w);
                        x = x.saturating_add(w + 1);
                        tab
                    })
                    .collect()
            }
        };

        let body_y = shelf_y.saturating_add(plan.shelf.height() + gap);
        let (left_w, right_x, right_w, friends_y) = if two_columns {
            let left_w = (content_w - GUTTER) / 2;
            let right_x = content_x + left_w + GUTTER;
            (left_w, right_x, content_w - left_w - GUTTER, body_y)
        } else {
            let friends_y = body_y.saturating_add(actions_h + gap);
            (content_w, content_x, content_w, friends_y)
        };
        let actions_heading = line(content_x, body_y, left_w);
        let friends_heading = line(right_x, friends_y, right_w);
        let friends_note = line(right_x, friends_y.saturating_add(1), right_w);
        let code_box = clip(Rect {
            x: content_x,
            y: body_y.saturating_add(body_h + gap),
            width: content_w,
            height: plan.field,
        });
        let input = if plan.field == 3 {
            clip(Rect {
                x: code_box.x + 2,
                y: code_box.y + 1,
                width: code_box.width.saturating_sub(4),
                height: 1,
            })
        } else {
            code_box
        };
        let mut placed = Vec::with_capacity(rows.len());
        let (mut action, mut friend) = (0u16, 0u16);
        for row in &rows {
            placed.push(match row {
                Row::Item(_) => {
                    action += 1;
                    line(content_x, body_y.saturating_add(action), left_w)
                }
                Row::Friend(_) => {
                    friend += 1;
                    line(right_x, friends_y.saturating_add(friend), right_w)
                }
                Row::Code => code_box,
            });
        }
        // Nothing may be clicked where the status bar or footer is drawn.
        let floor = status.y;
        let above = |r: Rect| {
            if r.bottom() <= floor {
                r
            } else {
                Rect::default()
            }
        };

        Self {
            header,
            name,
            shelf: plan.shelf,
            cards: cards.into_iter().map(above).collect(),
            actions_heading: above(actions_heading),
            friends_heading: above(friends_heading),
            friends_note: above(friends_note),
            rows: placed.into_iter().map(above).collect(),
            code_box: above(code_box),
            input: above(input),
            status,
            hint,
            footer,
        }
    }

    /// The row under a screen position, if any.
    #[must_use]
    pub fn row_at(&self, x: u16, y: u16) -> Option<usize> {
        self.rows.iter().position(|r| r.contains(Position { x, y }))
    }

    /// The game whose card or tab is under a screen position, if any.
    #[must_use]
    pub fn card_at(&self, x: u16, y: u16) -> Option<usize> {
        self.cards
            .iter()
            .position(|r| r.contains(Position { x, y }))
    }
}

pub fn draw_lobby(f: &mut Frame, lobby: &Lobby) {
    let rows = lobby.rows();
    let g = LobbyGeometry::new(f.area(), lobby);

    // Scenery first, so everything else is drawn over it.
    draw_stars(f.buffer_mut(), &g);
    draw_header(f, &g, lobby);
    draw_shelf(f, &g, lobby);

    let game = lobby.game();
    f.render_widget(
        Paragraph::new(heading(
            &format!("Play {}", game.name()),
            g.actions_heading.width,
        )),
        g.actions_heading,
    );
    f.render_widget(
        Paragraph::new(heading("Challenge a friend", g.friends_heading.width)),
        g.friends_heading,
    );
    if lobby.friends.is_empty() {
        f.render_widget(
            Paragraph::new(Line::styled(
                "  anyone you play turns up here",
                Style::default().fg(MUTED),
            )),
            g.friends_note,
        );
    }

    for (i, (&row, &area)) in rows.iter().zip(&g.rows).enumerate() {
        let chosen = i == lobby.selected && lobby.editing.is_none();
        if row == Row::Code {
            draw_code_box(f, &g, lobby, i == lobby.selected);
            continue;
        }
        if chosen {
            f.render_widget(Block::default().style(Style::default().bg(HIGHLIGHT)), area);
        }
        f.render_widget(
            Paragraph::new(row_line(lobby, row, chosen, area.width)),
            area,
        );
    }

    draw_status(f, &g, lobby, &rows);
    draw_footer(f, g.footer, lobby);

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

/// A heading over a column: its words, and a rule to the column's edge.
fn heading(text: &str, width: u16) -> Line<'static> {
    let rule = Style::default().fg(MUTED);
    let used = text.chars().count() + 4;
    Line::from(vec![
        Span::styled("── ", rule),
        Span::styled(
            text.to_string(),
            Style::default().fg(QUIET).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", "─".repeat(usize::from(width).saturating_sub(used))),
            rule,
        ),
    ])
}

fn draw_header(f: &mut Frame, g: &LobbyGeometry, lobby: &Lobby) {
    let title = Line::from(vec![
        Span::styled(
            " tui-tui",
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(MUTED),
        ),
    ]);
    f.render_widget(Paragraph::new(title), g.header);

    let muted = Style::default().fg(MUTED);
    if lobby.editing == Some(Field::Name) {
        // Room for the longest name, typed in place of the old one.
        let prompt = "name › ";
        let w = cells(prompt.chars().count() + NAME_MAX + 2);
        let area = Rect {
            x: g.header.right().saturating_sub(w),
            width: w.min(g.header.width),
            ..g.header
        };
        f.render_widget(Clear, area);
        let line = Line::from(vec![
            Span::styled(prompt, Style::default().fg(CURSOR)),
            Span::styled(lobby.name_input.clone(), Style::default().fg(BRIGHT)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        let at = prompt.chars().count() + lobby.name_input.chars().count();
        set_cursor(f, area, at);
        return;
    }
    if lobby.name.is_empty() {
        return;
    }
    let guest = if lobby.guest { " (guest)" } else { "" };
    let line = Line::from(vec![
        Span::styled("playing as ", muted),
        Span::styled(lobby.name.clone(), Style::default().fg(QUIET)),
        Span::styled(format!("{guest} "), muted),
    ]);
    f.render_widget(Paragraph::new(line.right_aligned()), g.name);
}

fn draw_shelf(f: &mut Frame, g: &LobbyGeometry, lobby: &Lobby) {
    for (i, (&kind, &area)) in Kind::ALL.iter().zip(&g.cards).enumerate() {
        let chosen = i == lobby.game % Kind::ALL.len();
        let name = if chosen {
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(QUIET)
        };
        let Shelf::Cards { thumbs } = g.shelf else {
            let style = if chosen {
                name.bg(HIGHLIGHT).fg(CURSOR)
            } else {
                name
            };
            f.render_widget(
                Paragraph::new(Line::styled(kind.name(), style).centered()).style(style),
                area,
            );
            continue;
        };

        let (border, kind_of) = if chosen {
            (CURSOR, BorderType::Thick)
        } else {
            (MUTED, BorderType::Rounded)
        };
        let block = Block::bordered()
            .border_type(kind_of)
            .border_style(Style::default().fg(border));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let mut y = inner.y;
        if thumbs {
            let pic = Rect {
                height: THUMB_H.min(inner.height),
                ..inner
            };
            kind.thumb(f.buffer_mut(), pic);
            y = y.saturating_add(THUMB_H);
        }
        let text = vec![
            Line::styled(kind.name(), name).centered(),
            Line::styled(kind.blurb(), Style::default().fg(MUTED)).centered(),
        ];
        let words = Rect {
            y,
            height: 2,
            ..inner
        }
        .intersection(inner);
        f.render_widget(Paragraph::new(text), words);
    }
    // Between the tabs.
    if g.shelf == Shelf::Tabs {
        for pair in g.cards.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.right() < b.x {
                let at = Position::new(a.right(), a.y);
                if f.area().contains(at) {
                    f.buffer_mut()[at].set_symbol("│").set_fg(MUTED);
                }
            }
        }
    }
}

/// What a row says: a marker if it is chosen, and the words.
fn row_line(lobby: &Lobby, row: Row, chosen: bool, width: u16) -> Line<'static> {
    let muted = Style::default().fg(MUTED);
    let label = if chosen {
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(QUIET)
    };
    let marker = if chosen {
        Span::styled(" ▸ ", Style::default().fg(CURSOR))
    } else {
        Span::raw("   ")
    };
    match row {
        Row::Item(item) => Line::from(vec![marker, Span::styled(item.label(), label)]),
        // Drawn by `draw_code_box`.
        Row::Code => Line::raw(""),
        Row::Friend(n) => {
            let Some(friend) = lobby.friends.get(n) else {
                return Line::raw("");
            };
            let games = match friend.games {
                1 => "1 game".to_string(),
                n => format!("{n} games"),
            };
            let about = format!("{games} · {} ", ago(friend.last_played));
            // The name takes what the numbers leave, the numbers going
            // first if there is not even room for a name.
            let about_w = about.chars().count();
            let room = usize::from(width).saturating_sub(3);
            let (about, name_w) = if room >= about_w + 6 {
                (about, room - about_w - 1)
            } else {
                (String::new(), room)
            };
            let name: String = friend.name.chars().take(name_w.min(20)).collect();
            let pad = room
                .saturating_sub(name.chars().count())
                .saturating_sub(about.chars().count());
            Line::from(vec![
                marker,
                Span::styled(name, label),
                Span::raw(" ".repeat(pad)),
                Span::styled(about, muted),
            ])
        }
    }
}

/// The code being typed, and how far along it the cursor sits. Without the
/// box's title to say what it is, the line says so itself.
fn code_line(lobby: &Lobby, boxed: bool) -> (Line<'static>, usize) {
    let muted = Style::default().fg(MUTED);
    let prompt = if boxed { "› " } else { "code › " };
    let start = prompt.chars().count();
    let prompt = Span::styled(prompt, Style::default().fg(CURSOR));
    if lobby.input.is_empty() {
        return (
            Line::from(vec![
                prompt,
                Span::styled("e.g. 42-tiger-marble-ocean", muted),
            ]),
            start,
        );
    }
    // The rest of the word Tab would fill in, greyed out after the cursor.
    let ghost = lobby.completion().map_or("", |word| {
        let typed = lobby.input.rsplit('-').next().map_or(0, str::len);
        &word[typed..]
    });
    (
        Line::from(vec![
            prompt,
            Span::styled(lobby.input.clone(), Style::default().fg(BRIGHT)),
            Span::styled(ghost, muted),
        ]),
        start + lobby.input.len(),
    )
}

/// How the code is looking, beside it.
fn code_verdict(lobby: &Lobby) -> Line<'static> {
    let muted = Style::default().fg(QUIET);
    match lobby.entry() {
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
    }
}

/// The box a code is typed into: bordered and titled when there is room, and
/// lit up when chosen or typed into.
fn draw_code_box(f: &mut Frame, g: &LobbyGeometry, lobby: &Lobby, chosen: bool) {
    let editing = lobby.editing == Some(Field::Code);
    let boxed = g.code_box.height >= 3;
    if boxed {
        let (colour, border) = if editing {
            (CURSOR, BorderType::Thick)
        } else if chosen && lobby.editing.is_none() {
            (CURSOR, BorderType::Rounded)
        } else {
            (QUIET, BorderType::Rounded)
        };
        let title = Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD);
        f.render_widget(Clear, g.code_box);
        f.render_widget(
            Block::bordered()
                .border_type(border)
                .border_style(Style::default().fg(colour))
                .title(Line::from(vec![
                    Span::raw(" "),
                    Span::styled("Join a game with a code", title),
                    Span::raw(" "),
                ])),
            g.code_box,
        );
    } else if editing || (chosen && lobby.editing.is_none()) {
        f.render_widget(
            Block::default().style(Style::default().bg(HIGHLIGHT)),
            g.code_box,
        );
    }

    let (line, cursor) = code_line(lobby, boxed);
    // Beside the code, how it is looking; or, before any is typed, where
    // one comes from.
    let aside = if editing {
        code_verdict(lobby)
    } else {
        Line::styled(
            "the code a friend got by hosting",
            Style::default().fg(MUTED),
        )
    };
    if verdict_fits(g, lobby) || (!editing && fits_beside(g, &line, &aside)) {
        f.render_widget(Paragraph::new(aside.right_aligned()), g.input);
    }
    f.render_widget(Paragraph::new(line), g.input);
    if editing {
        set_cursor(f, g.input, cursor);
    }
}

fn fits_beside(g: &LobbyGeometry, line: &Line, aside: &Line) -> bool {
    usize::from(g.input.width) >= line.width() + aside.width() + 2
}

/// Whether how the code is looking is shown in the code box, rather than in
/// the status bar.
fn verdict_fits(g: &LobbyGeometry, lobby: &Lobby) -> bool {
    lobby.editing == Some(Field::Code) && {
        let (line, _) = code_line(lobby, g.code_box.height >= 3);
        fits_beside(g, &line, &code_verdict(lobby))
    }
}

/// What the chosen row does, or whatever the lobby has to say.
fn status_line(g: &LobbyGeometry, lobby: &Lobby, rows: &[Row]) -> Line<'static> {
    let muted = Style::default().fg(QUIET);
    if let Some(i) = lobby.forgetting {
        let name = lobby.friends.get(i).map_or("them", |f| f.name.as_str());
        return Line::styled(
            format!("forget {name}? y to confirm, anything else keeps them"),
            Style::default().fg(CAPTURE),
        );
    }
    if lobby.editing == Some(Field::Name) {
        return Line::styled("what friends will see you as — enter saves", muted);
    }
    if lobby.editing == Some(Field::Code) {
        if !verdict_fits(g, lobby) {
            return code_verdict(lobby);
        }
        return Line::styled("your friend hosts, and their screen shows the code", muted);
    }
    if let Some(notice) = &lobby.notice {
        return Line::styled(notice.clone(), Style::default().fg(CURSOR));
    }
    if lobby.guest {
        return Line::styled(
            "a guest: another copy of tuitui has your profile open",
            muted,
        );
    }
    let game = lobby.game().name().to_lowercase();
    let text = match rows.get(lobby.selected) {
        Some(Row::Item(Item::Host)) => {
            format!("get a code for your opponent to type in, then wait here to play {game}")
        }
        Some(Row::Item(Item::Local)) if lobby.game().alone() => {
            format!("a game of {game} on your own")
        }
        Some(Row::Item(Item::Local)) => format!("two players taking turns at {game} here"),
        Some(Row::Friend(n)) => {
            let name = lobby.friends.get(*n).map_or("them", |f| f.name.as_str());
            format!("challenge {name} to a game of {game}")
        }
        Some(Row::Code) => {
            "join a friend who hosted: type or paste the code their screen shows".into()
        }
        None => String::new(),
    };
    Line::styled(text, muted)
}

fn draw_status(f: &mut Frame, g: &LobbyGeometry, lobby: &Lobby, rows: &[Row]) {
    if g.status.height >= 3 {
        f.render_widget(Clear, g.status);
        f.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED)),
            g.status,
        );
    }
    let line = status_line(g, lobby, rows);
    let line = if g.status.height >= 3 {
        line
    } else {
        line.centered()
    };
    f.render_widget(Paragraph::new(line), g.hint);
}

fn draw_footer(f: &mut Frame, area: Rect, lobby: &Lobby) {
    let tab = ("tab", "next");
    let on_code = lobby.rows().get(lobby.selected) == Some(&Row::Code);
    let mut keys: Vec<(&str, &str)> = if lobby.invite.is_some() {
        vec![("y", "accept"), ("n", "decline")]
    } else if lobby.editing == Some(Field::Code) {
        vec![
            ("enter", "join"),
            ("tab", "complete"),
            ("ctrl-u", "clear"),
            ("esc", "back"),
        ]
    } else if lobby.editing == Some(Field::Name) {
        vec![("enter", "save"), ("esc", "cancel")]
    } else if lobby.on_friend().is_some() {
        vec![
            ("enter", "challenge"),
            ("←/→", "game"),
            ("0-9", "join"),
            tab,
            ("x", "forget"),
            ("n", "rename"),
            ("q", "quit"),
        ]
    } else if on_code {
        vec![
            ("enter", "type a code"),
            ("←/→", "game"),
            tab,
            ("n", "rename"),
            ("q", "quit"),
        ]
    } else {
        vec![
            ("enter", "play"),
            ("←/→", "game"),
            tab,
            ("0-9", "join"),
            ("n", "rename"),
            ("↑/↓", "select"),
            ("q", "quit"),
        ]
    };
    if Kind::ALL.len() < 2 {
        keys.retain(|&(k, _)| k != "←/→");
    }
    let mut line = vec![Span::raw(" ")];
    line.extend(keycaps_fit(&keys, area.width.saturating_sub(1)));
    f.render_widget(Paragraph::new(Line::from(line)), area);
}

/// A scattering of stars in the sky between the header and the status bar,
/// clear of the shelf and the columns. Where they fall depends only on the
/// screen position, so they stay put from one frame to the next.
fn draw_stars(buf: &mut Buffer, g: &LobbyGeometry) {
    let content = g
        .cards
        .iter()
        .chain(&g.rows)
        .chain([&g.actions_heading, &g.friends_heading, &g.friends_note])
        .filter(|r| !r.is_empty())
        .copied()
        .reduce(Rect::union);
    // A margin round what is written, so no star crowds it.
    let keep_clear = content.map(|c| Rect {
        x: c.x.saturating_sub(4),
        y: c.y.saturating_sub(1),
        width: c.width.saturating_add(8),
        height: c.height.saturating_add(3),
    });
    let area = buf.area;
    let top = g.header.bottom().max(area.y);
    let bottom = g.status.y.min(area.bottom());
    for y in top..bottom {
        for x in area.left()..area.right() {
            let at = Position::new(x, y);
            if keep_clear.is_some_and(|k| k.contains(at)) {
                continue;
            }
            let (symbol, colour) = match hash(x, y) % 97 {
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
