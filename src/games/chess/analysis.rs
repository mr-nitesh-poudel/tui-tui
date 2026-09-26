//! Looking back over a game, with an engine's help: stepping through the
//! positions, the evaluation bar, the engine's best move, and a grade for
//! each move by how much it gave away.
//!
//! An engine's opinion is advice, so it is not on offer while a game against
//! someone else is still being played: only in hot-seat, where both players
//! are here, or once the game is decided. Looking back through the moves is
//! always allowed; the moves list shows them anyway.

use std::path::Path;
use std::time::{Duration, Instant};

use shakmaty::uci::UciMove;
use shakmaty::{Color, Move, Position};

use super::app::App;
use super::download::{self, Download, Progress};
use super::engine::{self, Engine, Score, Status};
use super::rules::Game;
use crate::games::Ctx;

/// How long the bar takes to settle on a new evaluation.
pub const BAR_EASE: Duration = Duration::from_millis(350);
/// The shallowest search trusted to grade a move by.
const GRADE_DEPTH: u32 = 10;

/// Where the bar is, and where it is heading.
#[derive(Clone, Copy, Debug)]
pub struct Bar {
    from: f32,
    to: f32,
    since: Option<Instant>,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            from: 0.5,
            to: 0.5,
            since: None,
        }
    }
}

/// How bad a move was, by how much of its side's chances it threw away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    Inaccuracy,
    Mistake,
    Blunder,
}

impl Grade {
    /// The chances a move lost, from 0 to 1, graded the way Lichess does.
    fn of(lost: f32) -> Option<Self> {
        match lost {
            l if l >= 0.30 => Some(Grade::Blunder),
            l if l >= 0.20 => Some(Grade::Mistake),
            l if l >= 0.10 => Some(Grade::Inaccuracy),
            _ => None,
        }
    }

    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            Grade::Inaccuracy => "?!",
            Grade::Mistake => "?",
            Grade::Blunder => "??",
        }
    }
}

/// How the engine is getting on, for the status line.
pub enum Thinking {
    Starting,
    Failed(String),
    /// The position is over, and needs no engine to score it.
    Decided(Score),
    Searching,
    Found {
        score: Score,
        depth: u32,
        best: Option<Move>,
    },
}

impl App {
    /// Whether the engine may be asked: in hot-seat, or once the game is
    /// decided.
    pub fn can_analyse(&self) -> bool {
        self.me.is_none() || !self.in_play()
    }

    /// Turns the engine on, starting it if there is one, or off, stopping it.
    /// With no engine to be found, offers to download one where there is a
    /// build for this machine. Pressed during a download, gives it up.
    pub fn toggle_analysis(&mut self, ctx: &mut Ctx) {
        if self.engine.take().is_some() {
            return;
        }
        if self.download.take().is_some() {
            ctx.note = Some("download cancelled".into());
            return;
        }
        if !self.can_analyse() {
            ctx.note = Some("analysis opens once the game is over".into());
            return;
        }
        let Some(path) = engine::locate() else {
            if download::this_build().is_some() {
                self.offer_download = true;
            } else {
                ctx.note = Some(engine::INSTALL_HINT.into());
            }
            return;
        };
        self.start_engine(&path, ctx);
    }

    fn start_engine(&mut self, path: &Path, ctx: &mut Ctx) {
        match Engine::start(path, ctx.waker.clone()) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.feed_engine();
            }
            Err(why) => ctx.note = Some(why),
        }
    }

    /// Fetches Stockfish, once the player has said yes to it.
    pub fn start_download(&mut self, ctx: &mut Ctx) {
        let (Some(build), Some(home)) = (download::this_build(), download::home()) else {
            ctx.note = Some(engine::INSTALL_HINT.into());
            return;
        };
        match Download::start(build, home, ctx.waker.clone()) {
            Ok(download) => self.download = Some(download),
            Err(why) => ctx.note = Some(why),
        }
    }

    /// Background work has news: a download that has finished starts the
    /// engine it brought, and one that failed says why.
    pub fn on_wake(&mut self, ctx: &mut Ctx) {
        let Some(progress) = self.download.as_ref().map(Download::progress) else {
            return;
        };
        match progress {
            Progress::Done(path) => {
                self.download = None;
                // The game may have moved on while it downloaded.
                if self.can_analyse() {
                    self.start_engine(&path, ctx);
                }
            }
            Progress::Failed(why) => {
                self.download = None;
                ctx.note = Some(why);
            }
            _ => {}
        }
    }

    /// The question the footer asks while offering a download.
    pub fn download_offer(&self) -> Option<String> {
        let build = download::this_build().filter(|_| self.offer_download)?;
        Some(format!(
            "no engine found. download Stockfish {} ({} MB)?",
            download::VERSION,
            build.size / 1_000_000
        ))
    }

    /// Tells the engine what is on the screen now, and what the whole game
    /// is, so it searches the one and then grades the other.
    pub fn feed_engine(&self) {
        let Some(engine) = &self.engine else { return };
        engine.focus(self.game.fen_at(self.shown_ply()));
        let game = (0..=self.game.plies())
            .filter(|&ply| !self.game.position_at(ply).is_game_over())
            .map(|ply| self.game.fen_at(ply))
            .collect();
        engine.grade(game);
    }

    /// How many moves in the position on the screen is.
    pub fn shown_ply(&self) -> usize {
        self.review
            .unwrap_or(self.game.plies())
            .min(self.game.plies())
    }

    /// The game as it stood at the position on the screen, when that is not
    /// the position now.
    pub fn reviewed(&self) -> Option<Game> {
        let ply = self.review?;
        (ply < self.game.plies()).then(|| self.game.at(ply))
    }

    /// Looks `delta` moves further on, or back if negative. Reaching the
    /// position now leaves looking back.
    pub fn step(&mut self, delta: isize) {
        let plies = self.game.plies();
        let ply = self.shown_ply().saturating_add_signed(delta).min(plies);
        self.review_at(ply);
    }

    /// Looks at the position `ply` moves in.
    pub fn review_at(&mut self, ply: usize) {
        self.review = (ply < self.game.plies()).then_some(ply);
        self.game.selected = None;
        self.feed_engine();
    }

    /// The score of the position `ply` moves in, and how deep it was found:
    /// straight from the rules where the game is over there, else from the
    /// engine if it has got to it.
    pub fn score_at(&self, ply: usize) -> Option<(Score, u32)> {
        let pos = self.game.position_at(ply);
        if pos.is_checkmate() {
            return Some((Score::mated(pos.turn() == Color::White), u32::MAX));
        }
        if pos.is_game_over() {
            return Some((Score::Cp(0), u32::MAX));
        }
        let eval = self.engine.as_ref()?.eval(&self.game.fen_at(ply))?;
        Some((eval.score, eval.depth))
    }

    /// How the `i`th move of the game went, once the engine has searched the
    /// positions either side of it well enough to say.
    pub fn grade(&self, i: usize) -> Option<Grade> {
        let (before, d1) = self.score_at(i)?;
        let (after, d2) = self.score_at(i + 1)?;
        if d1.min(d2) < GRADE_DEPTH {
            return None;
        }
        let white = self.game.position_at(i).turn() == Color::White;
        let chances = |s: Score| {
            let share = s.white_share();
            if white { share } else { 1.0 - share }
        };
        Grade::of(chances(before) - chances(after))
    }

    /// What the engine makes of the position on the screen.
    pub fn thinking(&self) -> Option<Thinking> {
        let engine = self.engine.as_ref()?;
        let ply = self.shown_ply();
        if let Some((score, u32::MAX)) = self.score_at(ply) {
            return Some(Thinking::Decided(score));
        }
        Some(match engine.status() {
            Status::Starting => Thinking::Starting,
            Status::Failed(why) => Thinking::Failed(why),
            Status::Ready => match engine.eval(&self.game.fen_at(ply)) {
                None => Thinking::Searching,
                Some(eval) => Thinking::Found {
                    score: eval.score,
                    depth: eval.depth,
                    best: eval.best.and_then(|uci| self.legal(ply, &uci)),
                },
            },
        })
    }

    /// The engine's best move in the position on the screen, to show on the
    /// board.
    pub fn best_move(&self) -> Option<Move> {
        match self.thinking()? {
            Thinking::Found { best, .. } => best,
            _ => None,
        }
    }

    /// A move the engine named, if it is one in that position: nothing it
    /// says is taken on trust.
    fn legal(&self, ply: usize, uci: &str) -> Option<Move> {
        uci.parse::<UciMove>()
            .ok()?
            .to_move(self.game.position_at(ply))
            .ok()
    }

    /// How full of white the bar is, from 0 to 1, easing towards the score
    /// of the position on the screen. Midway while there is no score yet.
    pub fn bar_share(&self) -> f32 {
        let target = self
            .score_at(self.shown_ply())
            .map_or(0.5, |(s, _)| s.white_share());
        let now = self.clock.unwrap_or_else(Instant::now);
        let mut bar = self.bar.get();
        let shown = bar.at(now);
        if (target - bar.to).abs() > f32::EPSILON {
            bar = Bar {
                from: shown,
                to: target,
                since: Some(now),
            };
            self.bar.set(bar);
        }
        bar.at(now)
    }

    /// Whether the bar is still on its way somewhere.
    pub fn bar_moving(&self) -> bool {
        let now = self.clock.unwrap_or_else(Instant::now);
        self.engine.is_some() && self.bar.get().since.is_some_and(|at| now < at + BAR_EASE)
    }
}

impl Bar {
    fn at(self, now: Instant) -> f32 {
        let Some(since) = self.since else {
            return self.to;
        };
        let t =
            (now.saturating_duration_since(since).as_secs_f32() / BAR_EASE.as_secs_f32()).min(1.0);
        // Quick to start and gentle to land.
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * eased
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_follow_how_much_was_thrown_away() {
        assert_eq!(Grade::of(0.05), None);
        assert_eq!(Grade::of(0.12), Some(Grade::Inaccuracy));
        assert_eq!(Grade::of(0.25), Some(Grade::Mistake));
        assert_eq!(Grade::of(0.6), Some(Grade::Blunder));
        assert_eq!(Grade::of(-0.3), None, "a move that gained is no mistake");
    }

    #[test]
    fn the_bar_eases_from_where_it_was() {
        let now = Instant::now();
        let bar = Bar {
            from: 0.5,
            to: 0.9,
            since: Some(now),
        };
        assert!((bar.at(now) - 0.5).abs() < 1e-6);
        let mid = bar.at(now + BAR_EASE / 2);
        assert!(mid > 0.7 && mid < 0.9, "{mid}");
        assert!((bar.at(now + BAR_EASE) - 0.9).abs() < 1e-6);
    }
}
