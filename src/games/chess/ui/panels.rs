//! Everything around the board: the players' cards, the moves and the
//! state of the game in the sidebar, the keys, the promotion prompt and the
//! verdict.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};
use shakmaty::san::SanPlus;
use shakmaty::{Color as Side, Piece, Position, Role};

use super::board::{canvas_dots, canvas_piece};
use super::pieces::piece_cell;
use super::{FLASH, Geometry, LIGHT, MATE_RED, PieceStyle, VERDICT_BG, side_name};
use crate::games::chess::analysis::{Grade, Thinking};
use crate::games::chess::app::{App, Ending, Finale, Tone};
use crate::games::chess::canvas;
use crate::games::chess::rules::PROMOTION_ROLES;
use crate::games::{Conn, Ctx, chrome};
use crate::ui::{BRIGHT, CAPTURE, CURSOR, MUTED, SELECTED, blend, cells, centred, keycaps};

/// Words that should be read, but are not the point.
const QUIET: Color = Color::Rgb(186, 182, 176);
/// Behind the latest move.
const HIGHLIGHT: Color = Color::Rgb(58, 54, 50);

/// A glow behind the state line, and the colour of its words, for each tone.
fn tone_style(tone: Tone) -> Style {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    match tone {
        Tone::Go => bold
            .bg(Color::Rgb(40, 58, 34))
            .fg(Color::Rgb(164, 216, 134)),
        Tone::Wait => Style::default().fg(MUTED),
        Tone::Offer | Tone::Warn => bold.bg(Color::Rgb(66, 52, 22)).fg(CURSOR),
        Tone::Alert => bold
            .bg(Color::Rgb(88, 26, 22))
            .fg(Color::Rgb(255, 128, 108)),
        Tone::Done => bold.bg(Color::Rgb(48, 44, 40)).fg(BRIGHT),
        Tone::Engine => bold
            .bg(Color::Rgb(26, 50, 74))
            .fg(Color::Rgb(156, 204, 246)),
    }
}

/// The sidebar: a card for each player, the side at the top of the board
/// above and the one at the bottom below, with the moves between them and
/// the state of the game just above the bottom card.
pub(super) fn draw_sidebar(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx) {
    let bottom_side = if app.flipped {
        Side::Black
    } else {
        Side::White
    };
    let inner_w = area.width.saturating_sub(4);
    let top = card(app, ctx, !bottom_side, inner_w);
    let bottom = card(app, ctx, bottom_side, inner_w);
    let [top_area, moves, state, bottom_area] = Layout::vertical([
        Constraint::Length(cells(top.lines.len()) + 2),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(cells(bottom.lines.len()) + 2),
    ])
    .areas(area);

    draw_card(f, top_area, top);
    draw_moves(f, moves, app);
    draw_state(f, state, app, ctx);
    draw_card(f, bottom_area, bottom);
}

/// One player's card, before it is drawn.
struct Card {
    title: String,
    /// Whose turn it is: the card is lit.
    lit: bool,
    dot: Option<Span<'static>>,
    lines: Vec<Line<'static>>,
}

fn card(app: &App, ctx: &Ctx, side: Side, width: u16) -> Card {
    let opponent = app.me.is_some_and(|me| me != side);
    let to_move = app.in_play() && app.game.turn() == side;
    let muted = Style::default().fg(MUTED);

    // With nobody there yet, the opponent's card is about getting them here.
    if opponent && !matches!(ctx.conn, Conn::Playing | Conn::Lost(_)) {
        let title = if ctx.conn == Conn::Waiting {
            "share this code"
        } else {
            "opponent"
        };
        return Card {
            title: title.into(),
            lit: ctx.conn == Conn::Waiting,
            dot: chrome::connection_dot(ctx),
            lines: chrome::connection_lines(ctx),
        };
    }

    let title = match app.me {
        None => side_name(side).to_string(),
        Some(_) if opponent => ctx.peer_label(),
        Some(_) => "you".into(),
    };
    let letters = matches!(app.piece_style, PieceStyle::Letter | PieceStyle::BigLetter);
    let king = figurine(Role::King, side, letters).to_string();
    // In hot-seat the card's title already names the side.
    let mut who = vec![Span::styled(king, Style::default().fg(BRIGHT))];
    if app.me.is_some() {
        who.push(Span::raw(format!(" {}", side_name(side))));
    }
    let turn = if to_move {
        vec![Span::styled(
            "to move",
            Style::default().fg(CURSOR).add_modifier(Modifier::BOLD),
        )]
    } else {
        vec![]
    };

    // What this side has taken, in the other side's pieces, and how far
    // ahead on material that leaves them.
    let taken: String = app
        .game
        .captured(!side)
        .into_iter()
        .map(|r| figurine(r, !side, letters))
        .collect();
    let edge = app.game.material_edge();
    let ahead = if side == Side::White { edge } else { -edge };
    let captures = if taken.is_empty() {
        vec![Span::styled("no captures", muted)]
    } else {
        vec![Span::styled(taken, Style::default().fg(QUIET))]
    };
    let lead = if ahead > 0 {
        vec![Span::styled(
            format!("+{ahead}"),
            Style::default().fg(SELECTED).add_modifier(Modifier::BOLD),
        )]
    } else {
        vec![]
    };

    let mut lines = vec![spread(who, turn, width), spread(captures, lead, width)];
    if opponent && let Conn::Lost(why) = &ctx.conn {
        lines.push(Line::styled(why.clone(), Style::default().fg(CAPTURE)));
    }
    Card {
        title,
        lit: to_move,
        dot: if opponent {
            chrome::connection_dot(ctx)
        } else {
            None
        },
        lines,
    }
}

fn draw_card(f: &mut Frame, area: Rect, card: Card) {
    let (border, title) = if card.lit {
        (
            Style::default().fg(CURSOR),
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        )
    } else {
        (Style::default().fg(MUTED), Style::default().fg(QUIET))
    };
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .padding(Padding::horizontal(1))
        .title(Line::styled(format!(" {} ", card.title), title));
    if let Some(dot) = card.dot {
        block = block.title(Line::from(vec![Span::raw(" "), dot, Span::raw(" ")]).right_aligned());
    }
    f.render_widget(
        Paragraph::new(card.lines)
            .wrap(Wrap { trim: true })
            .block(block),
        area,
    );
}

/// `left` at the start of a line `width` wide and `right` at its end, or
/// straight after `left` if there is no room between.
fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let used = Line::from(left.clone()).width() + Line::from(right.clone()).width();
    let gap = usize::from(width).saturating_sub(used).max(1);
    let mut spans = left;
    if !right.is_empty() {
        spans.push(Span::raw(" ".repeat(gap)));
        spans.extend(right);
    }
    Line::from(spans)
}

/// A piece as one character: a figurine, outlined for white and solid for
/// black, or its letter where figurines come out double width.
fn figurine(role: Role, side: Side, letters: bool) -> char {
    if letters {
        let c = role.upper_char();
        return if side == Side::White {
            c
        } else {
            c.to_ascii_lowercase()
        };
    }
    let white = side == Side::White;
    match role {
        Role::King => {
            if white {
                '♔'
            } else {
                '♚'
            }
        }
        Role::Queen => {
            if white {
                '♕'
            } else {
                '♛'
            }
        }
        Role::Rook => {
            if white {
                '♖'
            } else {
                '♜'
            }
        }
        Role::Bishop => {
            if white {
                '♗'
            } else {
                '♝'
            }
        }
        Role::Knight => {
            if white {
                '♘'
            } else {
                '♞'
            }
        }
        Role::Pawn => {
            if white {
                '♙'
            } else {
                '♟'
            }
        }
    }
}

/// A move as written in the list: its piece letters as figurines of the
/// side that made it, unless the pieces are being drawn as letters.
fn written(san: &str, side: Side, letters: bool) -> String {
    if letters {
        return san.to_string();
    }
    san.chars()
        .map(|c| match Role::from_char(c) {
            Some(role) if c.is_ascii_uppercase() => figurine(role, side, false),
            _ => c,
        })
        .collect()
}

/// The score and how it came about, once the game is decided.
fn result(app: &App) -> Option<(&'static str, &'static str)> {
    let win = |winner: Side| {
        if winner == Side::White {
            "1–0"
        } else {
            "0–1"
        }
    };
    let pos = &app.game.pos;
    if let Some(loser) = app.game.resigned {
        Some((win(!loser), "resignation"))
    } else if pos.is_checkmate() {
        Some((win(!app.game.turn()), "checkmate"))
    } else if app.draw_agreed {
        Some(("½–½", "agreed"))
    } else if pos.is_stalemate() {
        Some(("½–½", "stalemate"))
    } else if pos.is_insufficient_material() {
        Some(("½–½", "insufficient material"))
    } else {
        None
    }
}

fn draw_moves(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
        .padding(Padding::horizontal(1))
        .title(Line::styled(" moves ", Style::default().fg(QUIET)));
    let inner_h = usize::from(area.height.saturating_sub(2));
    let muted = Style::default().fg(MUTED);
    let latest = Style::default()
        .fg(CURSOR)
        .bg(HIGHLIGHT)
        .add_modifier(Modifier::BOLD);
    let letters = matches!(app.piece_style, PieceStyle::Letter | PieceStyle::BigLetter);
    // The move into the position on the screen.
    let marked = app.shown_ply().checked_sub(1);
    let grades = app.engine.is_some();

    let mut lines: Vec<Line> = app
        .game
        .history
        .chunks(2)
        .enumerate()
        .map(|(i, pair)| {
            let mut spans = vec![Span::styled(format!("{:>3}. ", i + 1), muted)];
            for (j, san) in pair.iter().enumerate() {
                let side = if j == 0 { Side::White } else { Side::Black };
                let ply = 2 * i + j;
                let text = written(san, side, letters);
                let style = if Some(ply) == marked {
                    latest
                } else {
                    Style::default().fg(QUIET)
                };
                let mut width = text.chars().count();
                spans.push(Span::styled(text, style));
                if let Some(grade) = app.grade(ply).filter(|_| grades) {
                    width += grade.symbol().len();
                    spans.push(Span::styled(grade.symbol(), grade_style(grade)));
                }
                if j == 0 {
                    let pad = 10usize.saturating_sub(width);
                    spans.push(Span::raw(" ".repeat(pad)));
                }
            }
            Line::from(spans)
        })
        .collect();
    if lines.is_empty() {
        lines.push(Line::styled("no moves yet", muted));
    }
    if let Some((score, how)) = result(app) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("     {score}"),
                Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {how}"), muted),
        ]));
    }

    // Keep the tail of the game visible, or the move being looked back at,
    // and say how much is above it.
    if lines.len() > inner_h && inner_h > 1 {
        let end = match (app.review, marked) {
            (Some(_), Some(ply)) => (ply / 2 + 1).max(inner_h - 1).min(lines.len()),
            (Some(_), None) => inner_h - 1,
            (None, _) => lines.len(),
        };
        let hidden = end - (inner_h - 1);
        lines.truncate(end);
        lines.drain(..hidden);
        if hidden > 0 {
            lines.insert(0, Line::styled(format!("↑ {hidden} earlier"), muted));
        } else {
            lines.truncate(inner_h);
        }
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Where the game stands, on a band of colour that says how much it
/// matters. A draw offer carries the key that accepts it.
fn draw_state(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx) {
    let (text, tone) = analysis_line(app).unwrap_or_else(|| app.state_line(ctx));
    let style = tone_style(tone);
    let text = if tone == Tone::Alert {
        text.to_uppercase()
    } else {
        text
    };
    let mut spans = vec![Span::styled(format!(" {text} "), style)];
    if tone == Tone::Offer {
        spans.push(Span::raw("  "));
        spans.extend(keycaps(&[("d", "accept")]));
    }
    f.render_widget(Paragraph::new(Line::from(spans).centered()), area);
}

/// What the engine makes of the position on the screen, and where that
/// position is when looking back. `None` to say how the game stands instead.
fn analysis_line(app: &App) -> Option<(String, Tone)> {
    if let Some(download) = &app.download {
        return Some((download.progress().describe(), Tone::Wait));
    }
    let plies = app.game.plies();
    let at = app
        .review
        .map(|ply| format!("{ply}/{plies} · "))
        .unwrap_or_default();
    let (text, tone) = match app.thinking() {
        None | Some(Thinking::Decided(_)) => {
            let ply = app.review?;
            return Some((format!("move {ply} of {plies}"), Tone::Wait));
        }
        Some(Thinking::Starting) => ("starting the engine…".into(), Tone::Wait),
        Some(Thinking::Failed(why)) => (why, Tone::Warn),
        Some(Thinking::Searching) => ("thinking…".into(), Tone::Wait),
        Some(Thinking::Found { score, depth, best }) => {
            let letters = matches!(app.piece_style, PieceStyle::Letter | PieceStyle::BigLetter);
            let pos = app.game.position_at(app.shown_ply());
            let best = best.map_or(String::new(), |m| {
                let san = SanPlus::from_move(pos.clone(), m).to_string();
                format!("  best {}", written(&san, pos.turn(), letters))
            });
            (
                format!("{}{best}  depth {depth}", score.signed()),
                Tone::Engine,
            )
        }
    };
    Some((format!("{at}{text}"), tone))
}

fn grade_style(grade: Grade) -> Style {
    let colour = match grade {
        Grade::Inaccuracy => Color::Rgb(232, 200, 92),
        Grade::Mistake => Color::Rgb(236, 142, 60),
        Grade::Blunder => Color::Rgb(232, 64, 52),
    };
    Style::default().fg(colour).add_modifier(Modifier::BOLD)
}

pub(super) fn draw_footer(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx) {
    let resign: &[(&str, &str)] = &[("y", "resign"), ("n", "cancel")];
    let fetch: &[(&str, &str)] = &[("y", "download"), ("n", "not now")];
    let offer = app.download_offer();
    let question = match &offer {
        Some(text) => Some((text.as_str(), fetch)),
        None => app.confirm_resign.then_some(("resign this game?", resign)),
    };
    let analyse = if app.download.is_some() {
        "cancel download"
    } else if app.engine.is_some() {
        "stop analysing"
    } else {
        "analyse"
    };
    let over = !app.in_play();
    // The most useful first, as the narrowest screens keep only those.
    let keys: Vec<(&str, &str)> = if app.game.promotion.is_some() {
        vec![
            ("click/←→", "choose"),
            ("enter", "promote"),
            ("esc", "cancel"),
        ]
    } else if app.review.is_some() {
        let mut keys = vec![(if over { "←/→" } else { ",/." }, "step")];
        if app.can_analyse() {
            keys.push(("a", analyse));
        }
        keys.push(("esc", "back to the game"));
        keys
    } else if over {
        vec![
            ("←/→", "replay"),
            ("a", analyse),
            ("f", "flip"),
            ("p", "pieces"),
            ("m", "mouse"),
            ("q", "lobby"),
        ]
    } else {
        let mut keys = vec![("click", "move"), ("r", "resign"), ("d", "draw")];
        if ctx.has_chat() {
            keys.push(("t", "chat"));
        }
        if app.can_analyse() {
            keys.push(("a", analyse));
        }
        keys.extend([
            ("f", "flip"),
            ("p", "pieces"),
            ("m", "mouse"),
            ("q", "lobby"),
        ]);
        keys
    };
    chrome::footer(f, area, ctx, question, &keys);
}

pub(super) fn draw_promotion(f: &mut Frame, g: &Geometry, app: &App) {
    let Some(p) = &app.game.promotion else { return };
    let side = app.game.turn();
    f.render_widget(Clear, g.promo);

    let inner_h = g.promo.height.saturating_sub(2);
    let mut rows: Vec<Line> = Vec::new();
    for sub in 0..inner_h {
        let mut spans = vec![Span::raw(" ")];
        for (i, role) in PROMOTION_ROLES.iter().enumerate() {
            let bg = if i == p.choice { CURSOR } else { LIGHT };
            let piece = Piece {
                color: side,
                role: *role,
            };
            spans.extend(piece_cell(
                piece,
                sub,
                g.promo_cell,
                inner_h,
                app.piece_style,
                bg,
            ));
        }
        rows.push(Line::from(spans));
    }

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CURSOR))
        .title(Line::from(" promote to "));
    f.render_widget(
        Paragraph::new(rows).alignment(Alignment::Left).block(block),
        g.promo,
    );

    if let Some(dots) = canvas_dots(app.piece_style, g.promo_cell, inner_h) {
        // Past the border and the one-cell margin the rows start with.
        let area = Rect {
            x: g.promo.x + 2,
            y: g.promo.y + 1,
            width: 4 * g.promo_cell,
            height: inner_h,
        };
        let pieces: Vec<_> = PROMOTION_ROLES
            .iter()
            .enumerate()
            .map(|(i, &role)| {
                let piece = Piece { color: side, role };
                canvas_piece(piece, (cells(i), 0), (0.0, 0.0), (g.promo_cell, inner_h))
            })
            .collect();
        canvas::stamp(f.buffer_mut(), area, &pieces, dots);
    }
}

/// The letters of CHECKMATE and RESIGNED, five blocks square.
fn glyph(c: char) -> [&'static str; 5] {
    match c {
        'C' => [" ████", "█    ", "█    ", "█    ", " ████"],
        'H' => ["█   █", "█   █", "█████", "█   █", "█   █"],
        'E' => ["█████", "█    ", "████ ", "█    ", "█████"],
        'K' => ["█   █", "█  █ ", "███  ", "█  █ ", "█   █"],
        'M' => ["█   █", "██ ██", "█ █ █", "█   █", "█   █"],
        'A' => [" ███ ", "█   █", "█████", "█   █", "█   █"],
        'T' => ["█████", "  █  ", "  █  ", "  █  ", "  █  "],
        'R' => ["████ ", "█   █", "████ ", "█  █ ", "█   █"],
        'S' => [" ████", "█    ", " ███ ", "    █", "████ "],
        'I' => ["█████", "  █  ", "  █  ", "  █  ", "█████"],
        'G' => [" ████", "█    ", "█  ██", "█   █", " ████"],
        'N' => ["█   █", "██  █", "█ █ █", "█  ██", "█   █"],
        'D' => ["████ ", "█   █", "█   █", "█   █", "████ "],
        _ => ["     "; 5],
    }
}

/// The verdict across the middle of the board, spelled out a letter at a
/// time as `reveal` goes from 0 to 1. Each letter lands white hot and cools
/// to red.
pub(super) fn draw_verdict(
    f: &mut Frame,
    g: &Geometry,
    app: &App,
    ctx: &Ctx,
    fin: &Finale,
    reveal: f32,
) {
    let word = match fin.how {
        Ending::Checkmate => "CHECKMATE",
        Ending::Resignation => "RESIGNED",
    };
    let letters = f32::from(cells(word.len()));
    let shown = reveal * letters;
    let colour = |i: usize| {
        let age = shown - f32::from(cells(i));
        if age < 1.0 {
            blend(FLASH, MATE_RED, age)
        } else {
            MATE_RED
        }
    };

    let big_w = cells(word.len()) * 6 - 1;
    let big = g.board.width >= big_w + 6 && g.board.height >= 13;
    let mut lines = vec![Line::raw("")];
    if big {
        for row in 0..5 {
            let mut spans = Vec::new();
            for (i, c) in word.chars().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                let text = if f32::from(cells(i)) < shown {
                    glyph(c)[row]
                } else {
                    "     "
                };
                spans.push(Span::styled(text, Style::default().fg(colour(i))));
            }
            lines.push(Line::from(spans).centered());
        }
    } else {
        let spans: Vec<Span> = word
            .chars()
            .enumerate()
            .map(|(i, c)| {
                let text = if f32::from(cells(i)) < shown { c } else { ' ' };
                Span::styled(format!("{text} "), Style::default().fg(colour(i)).bold())
            })
            .collect();
        lines.push(Line::from(spans).centered());
    }
    lines.push(Line::raw(""));

    let done = reveal >= 1.0;
    if done {
        let loser = app.game.resigned.unwrap_or(app.game.turn());
        let verdict = match (fin.how, app.me) {
            (Ending::Checkmate, Some(me)) if me == loser => {
                format!("{} wins", ctx.peer_label())
            }
            (Ending::Checkmate, Some(_)) => "you win".to_string(),
            (Ending::Checkmate, None) => format!("{} wins", side_name(!loser)),
            (Ending::Resignation, Some(me)) if me == loser => "you resigned".to_string(),
            (Ending::Resignation, Some(_)) => {
                format!("{} resigned · you win", ctx.peer_label())
            }
            (Ending::Resignation, None) => {
                format!("{} resigns · {} wins", side_name(loser), side_name(!loser))
            }
        };
        lines.push(Line::styled(verdict, Style::default().fg(FLASH).bold()).centered());
        lines.push(Line::styled("any key to see the board", Style::default().fg(MUTED)).centered());
    }

    let width = if big {
        big_w + 6
    } else {
        2 * cells(word.len()) + 8
    }
    .max(30);
    let height = cells(lines.len().max(if big { 10 } else { 5 })) + 2;
    // In the half of the board away from the king, so the mate stays in
    // view, if there is room there.
    let half = g.grid.height / 2;
    let king_row = app.finale().map_or(0, |fin| {
        let row = 7 - fin.king.rank() as u16;
        if app.flipped { 7 - row } else { row }
    });
    let away = Rect {
        x: g.board.x,
        y: if king_row < 4 {
            g.grid.y + half
        } else {
            g.grid.y
        },
        width: g.board.width,
        height: half,
    };
    let area = centred(if height <= half { away } else { g.board }, width, height);
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(MATE_RED))
        .style(Style::default().bg(VERDICT_BG));
    f.render_widget(Paragraph::new(lines).block(block), area);
}
