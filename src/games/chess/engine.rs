//! A chess engine to analyse with: Stockfish, or anything else that speaks
//! UCI, run as a separate program.
//!
//! The engine is the player's own. It is looked for on the `PATH` and where
//! package managers put it, or wherever `TUITUI_ENGINE` says. It is never
//! built in: Stockfish is GPL, and a separate program keeps it that way
//! without it touching this crate's licence.
//!
//! One task owns the process and talks to it. What it finds goes into a table
//! of evaluations shared with the game, keyed by position, and the main loop
//! is woken to draw it. The position on the screen is searched first, and
//! deeply; after that every position of the game is searched more quickly,
//! which is what grades the moves.
//!
//! Nothing it prints is trusted to be well formed. A line that does not parse
//! is skipped, a score out of any sensible range is held to one, a line too
//! long to be real ends the conversation, and an engine that goes quiet or
//! dies only stops analysis.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::games::Waker;
use crate::session::lines::Lines;

/// The environment variable that names an engine to use instead.
pub const ENGINE_VAR: &str = "TUITUI_ENGINE";
/// How to get one, for when there is none.
pub const INSTALL_HINT: &str = "no engine found: install stockfish, or set TUITUI_ENGINE";

/// How deep the position on the screen is searched, and for how long at
/// most, and the same for the rest of the game's positions.
const FOCUS: Limits = Limits {
    depth: 20,
    millis: 4000,
};
const GRADE: Limits = Limits {
    depth: 13,
    millis: 700,
};
/// How long the engine has to say hello before it is given up on.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// How long the engine has to leave once told to.
const LEAVE: Duration = Duration::from_millis(500);
/// How long a line to the engine may take to go. An engine that has stopped
/// reading would otherwise hold the task, and its own process, forever.
const WRITE: Duration = Duration::from_secs(5);
/// Beyond this a mate is scored the same, however far off it is.
const MATE: i32 = 100_000;
/// No real mate is further off than this. A mate the engine puts further
/// is taken to be this far, which keeps every sum on a score in range.
const MATE_MAX: i32 = 1000;

#[derive(Clone, Copy)]
struct Limits {
    depth: u32,
    millis: u32,
}

/// How good a position is for white.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Score {
    /// In hundredths of a pawn.
    Cp(i32),
    /// Mate in this many moves: white mates if positive, black if negative.
    /// Zero is a position already mated, which [`Score::mated`] makes.
    Mate(i32),
}

impl Score {
    /// The score of a position that is checkmate, `white_to_move` being who
    /// has been mated.
    /// Mate in 0 has no sign to say who won, so it is scored as a mate
    /// already delivered: as good as it gets for the side that did it.
    #[must_use]
    pub fn mated(white_to_move: bool) -> Self {
        Score::Cp(if white_to_move { -MATE } else { MATE })
    }

    /// In hundredths of a pawn, with a mate counting for more than any
    /// material, and a nearer mate for more than a farther one.
    #[must_use]
    pub fn centipawns(self) -> i32 {
        match self {
            Score::Cp(cp) => cp,
            Score::Mate(n) if n > 0 => MATE - n,
            Score::Mate(n) => -MATE - n,
        }
    }

    /// White's chances, from 0 to 1: how full the bar is. The curve is the
    /// one Lichess fitted to how games actually turn out.
    #[must_use]
    pub fn white_share(self) -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "clamped to a mate's score, which f32 holds exactly"
        )]
        let cp = self.centipawns().clamp(-MATE, MATE) as f32;
        1.0 / (1.0 + (-0.003_682_08 * cp).exp())
    }

    /// The score as it is written on the bar: `1.3`, `12`, `M4`, with no sign,
    /// since the end of the bar it is written at says who is ahead.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Score::Mate(n) => format!("M{}", n.abs()),
            Score::Cp(cp) if cp.abs() >= MATE => "#".into(),
            Score::Cp(cp) if cp.abs() >= 995 => format!("{}", (cp.abs() + 50) / 100),
            Score::Cp(cp) => format!("{:.1}", f64::from(cp.abs()) / 100.0),
        }
    }

    /// The score with its sign, for the sidebar: `+1.3`, `-0.4`, `-M3`.
    #[must_use]
    pub fn signed(self) -> String {
        match self {
            Score::Mate(n) if n < 0 => format!("-M{}", -n),
            Score::Mate(n) => format!("M{n}"),
            Score::Cp(cp) if cp.abs() >= MATE => "#".into(),
            Score::Cp(cp) => format!("{:+.1}", f64::from(cp) / 100.0),
        }
    }

    /// Whether white is ahead, or level.
    #[must_use]
    pub fn white_ahead(self) -> bool {
        self.centipawns() >= 0
    }
}

/// What the engine made of one position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eval {
    pub depth: u32,
    pub score: Score,
    /// The move it would play, in UCI.
    pub best: Option<String>,
}

/// How the engine is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready,
    Failed(String),
}

/// What the game asks of the engine.
enum Ask {
    /// Search this position, before anything else.
    Focus(String),
    /// Then these, the whole game in order, for grading its moves.
    Grade(Vec<String>),
}

/// Positions and what the engine made of them, shared with the game.
pub type Evals = Arc<Mutex<HashMap<String, Eval>>>;

/// A running engine. Dropping it stops the task, which kills the process.
pub struct Engine {
    asks: UnboundedSender<Ask>,
    evals: Evals,
    status: Arc<Mutex<Status>>,
}

impl Engine {
    /// Starts the engine at `path`, calling `wake` whenever there is
    /// something new to draw. Fails at once if there is no async runtime to
    /// run it on, or the program cannot be started; anything later shows up
    /// in [`Engine::status`].
    ///
    /// # Errors
    ///
    /// If there is no async runtime, or the program cannot be started.
    pub fn start(path: &Path, wake: Option<Waker>) -> Result<Self, String> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "analysis needs the async runtime".to_string())?;
        let _guard = runtime.enter();
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", path.display()))?;
        let (asks, asked) = unbounded_channel();
        let engine = Self {
            asks,
            evals: Arc::default(),
            status: Arc::new(Mutex::new(Status::Starting)),
        };
        let (evals, status) = (engine.evals.clone(), engine.status.clone());
        let wake = wake.unwrap_or_else(|| Arc::new(|| {}));
        runtime.spawn(async move {
            let result = drive(&mut child, asked, &evals, &status, &wake).await;
            // However it ended, the process ends too, and is waited for so
            // it does not linger as a zombie: a moment to leave on its own
            // after `quit`, and then it is killed.
            if tokio::time::timeout(LEAVE, child.wait()).await.is_err() {
                let _ = child.kill().await;
            }
            if let Err(why) = result {
                *lock(&status) = Status::Failed(why);
                wake();
            }
        });
        Ok(engine)
    }

    /// Search this position first, and deeply.
    pub fn focus(&self, fen: String) {
        let _ = self.asks.send(Ask::Focus(fen));
    }

    /// Then search each of these, the whole game, to grade its moves.
    pub fn grade(&self, fens: Vec<String>) {
        let _ = self.asks.send(Ask::Grade(fens));
    }

    #[must_use]
    pub fn eval(&self, fen: &str) -> Option<Eval> {
        lock(&self.evals).get(fen).cloned()
    }

    #[must_use]
    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }
}

/// A poisoned lock only means a panic elsewhere mid-update, and what it holds
/// is still usable.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Where an engine is, if there is one: `TUITUI_ENGINE` if it is set, else
/// `stockfish` on the `PATH` or where package managers put it, else one
/// downloaded earlier.
pub fn locate() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os(ENGINE_VAR).filter(|v| !v.is_empty()) {
        let named = PathBuf::from(named);
        // A bare name is looked up like any other command.
        if named.components().count() > 1 {
            return Some(named);
        }
        return search(named.as_os_str());
    }
    search("stockfish".as_ref()).or_else(super::download::installed)
}

fn search(name: &std::ffi::OsStr) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    // Homebrew's and Debian's places, for when the PATH is thin.
    let usual = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/games",
        "/usr/bin",
    ];
    std::env::split_paths(&path)
        .chain(usual.iter().map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// Talks to the engine until the game lets go of it.
async fn drive(
    child: &mut Child,
    mut asked: UnboundedReceiver<Ask>,
    evals: &Evals,
    status: &Arc<Mutex<Status>>,
    wake: &Waker,
) -> Result<(), String> {
    let mut stdin = child.stdin.take().ok_or("the engine has no input")?;
    let stdout = child.stdout.take().ok_or("the engine has no output")?;
    // Bounded, as a peer's lines are, and safe to race the game's asks.
    let mut lines = Lines::new(stdout);
    let gone = |e: std::io::Error| format!("the engine stopped: {e}");

    // Say hello, and wait to hear it back.
    send(&mut stdin, "uci").await?;
    let hello = async {
        while let Some(line) = lines.next_line().await.map_err(gone)? {
            if line.trim() == "uciok" {
                return Ok(());
            }
        }
        Err("the engine quit before it was ready".to_string())
    };
    tokio::time::timeout(HANDSHAKE, hello)
        .await
        .map_err(|_| "the engine did not answer".to_string())??;
    let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 4));
    send(
        &mut stdin,
        &format!("setoption name Threads value {threads}"),
    )
    .await?;
    *lock(status) = Status::Ready;
    wake();

    let mut focus: Option<String> = None;
    let mut queue: Vec<String> = Vec::new();
    // How deep each position has been searched to the end.
    let mut searched: HashMap<String, u32> = HashMap::new();
    // The search running now, and whether it has been told to stop.
    let mut running: Option<(String, Limits)> = None;
    let mut stopping = false;

    loop {
        if running.is_none() {
            let next = pick(focus.as_deref(), &queue, &searched);
            if let Some((fen, limits)) = next {
                send(&mut stdin, &format!("position fen {fen}")).await?;
                send(
                    &mut stdin,
                    &format!("go depth {} movetime {}", limits.depth, limits.millis),
                )
                .await?;
                running = Some((fen, limits));
            }
        }

        tokio::select! {
            ask = asked.recv() => match ask {
                // The game has let go: ask the engine to leave, and the
                // process is killed on the way out either way.
                None => {
                    let _ = send(&mut stdin, "quit").await;
                    return Ok(());
                }
                Some(Ask::Focus(fen)) => {
                    let elsewhere = running.as_ref().is_some_and(|(r, _)| *r != fen);
                    focus = Some(fen);
                    if elsewhere && !stopping {
                        send(&mut stdin, "stop").await?;
                        stopping = true;
                    }
                }
                Some(Ask::Grade(fens)) => queue = fens,
            },
            line = lines.next_line() => {
                let line = line.map_err(gone)?.ok_or("the engine quit")?;
                let Some((fen, limits)) = &running else { continue };
                match parse(&line) {
                    Some(Said::Info { depth, score, best }) => {
                        let score = white_relative(score, fen);
                        let changed = {
                            let mut evals = lock(evals);
                            let known = evals.get(fen);
                            let deeper = known.is_none_or(|e| depth >= e.depth);
                            if deeper {
                                let best = best.or_else(|| known.and_then(|e| e.best.clone()));
                                evals.insert(fen.clone(), Eval { depth, score, best });
                            }
                            deeper
                        };
                        if changed {
                            wake();
                        }
                    }
                    Some(Said::BestMove(best)) => {
                        // A search that was stopped did not finish.
                        if !stopping {
                            searched.insert(fen.clone(), limits.depth);
                        }
                        if let Some(best) = best {
                            let mut evals = lock(evals);
                            if let Some(e) = evals.get_mut(fen) {
                                e.best = Some(best);
                            }
                        }
                        running = None;
                        stopping = false;
                        wake();
                    }
                    None => {}
                }
            }
        }
    }
}

/// The position on the screen if it has not been searched deeply yet, else
/// the first of the game's positions not yet searched for grading.
fn pick(
    focus: Option<&str>,
    queue: &[String],
    searched: &HashMap<String, u32>,
) -> Option<(String, Limits)> {
    let done = |fen: &str, limits: Limits| searched.get(fen).is_some_and(|&d| d >= limits.depth);
    if let Some(fen) = focus
        && !done(fen, FOCUS)
    {
        return Some((fen.to_string(), FOCUS));
    }
    queue
        .iter()
        .find(|fen| !done(fen, GRADE))
        .map(|fen| (fen.clone(), GRADE))
}

async fn send(stdin: &mut ChildStdin, line: &str) -> Result<(), String> {
    let write = async {
        stdin.write_all(format!("{line}\n").as_bytes()).await?;
        stdin.flush().await
    };
    match tokio::time::timeout(WRITE, write).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("the engine stopped listening: {e}")),
        Err(_) => Err("the engine stopped listening".into()),
    }
}

/// A line from the engine worth acting on.
#[derive(Debug, PartialEq, Eq)]
enum Said {
    /// How the search is going, with the score for the side to move.
    Info {
        depth: u32,
        score: Score,
        best: Option<String>,
    },
    /// The search is over.
    BestMove(Option<String>),
}

/// `info … depth 18 … score cp 34 … pv e2e4 e7e5` and `bestmove e2e4`. An
/// info line without a depth and an exact score says nothing to act on, and
/// neither does one for a line other than the best.
fn parse(line: &str) -> Option<Said> {
    let mut words = line.split_whitespace();
    match words.next()? {
        "bestmove" => {
            let best = words.next().filter(|m| *m != "(none)").map(str::to_string);
            Some(Said::BestMove(best))
        }
        "info" => {
            let (mut depth, mut score, mut best) = (None, None, None);
            while let Some(word) = words.next() {
                match word {
                    "depth" => depth = words.next()?.parse().ok(),
                    "multipv" if words.next()? != "1" => return None,
                    "score" => {
                        let kind = words.next()?;
                        let n: i32 = words.next()?.parse().ok()?;
                        // Held in range, so no score can overflow when it
                        // is turned round or added to: release builds check.
                        score = Some(match kind {
                            "cp" => Score::Cp(n.clamp(-MATE, MATE)),
                            "mate" => Score::Mate(n.clamp(-MATE_MAX, MATE_MAX)),
                            _ => return None,
                        });
                    }
                    // A bound is not a score, only a limit on one.
                    "lowerbound" | "upperbound" => return None,
                    "pv" => {
                        best = words.next().map(str::to_string);
                        break;
                    }
                    _ => {}
                }
            }
            Some(Said::Info {
                depth: depth?,
                score: score?,
                best,
            })
        }
        _ => None,
    }
}

/// The engine scores for the side to move; the bar is for white.
fn white_relative(score: Score, fen: &str) -> Score {
    let black_to_move = fen.split_whitespace().nth(1) == Some("b");
    match (score, black_to_move) {
        (s, false) => s,
        (Score::Cp(cp), true) => Score::Cp(-cp),
        (Score::Mate(n), true) => Score::Mate(-n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_lines_give_a_depth_a_score_and_a_move() {
        assert_eq!(
            parse("info depth 18 seldepth 24 multipv 1 score cp 34 nodes 1 pv e2e4 e7e5"),
            Some(Said::Info {
                depth: 18,
                score: Score::Cp(34),
                best: Some("e2e4".into())
            })
        );
        assert_eq!(
            parse("info depth 9 score mate -3 pv h7h8q"),
            Some(Said::Info {
                depth: 9,
                score: Score::Mate(-3),
                best: Some("h7h8q".into())
            })
        );
        assert_eq!(
            parse("bestmove e2e4 ponder e7e5"),
            Some(Said::BestMove(Some("e2e4".into())))
        );
        assert_eq!(parse("bestmove (none)"), Some(Said::BestMove(None)));
    }

    #[test]
    fn anything_else_is_ignored() {
        for line in [
            "info string NNUE enabled",
            "info depth 10 score cp 12 lowerbound pv e2e4",
            "info depth 10 multipv 2 score cp 12 pv d2d4",
            "info depth x score cp 12",
            "info depth 10 score cp",
            "info depth 10 score wat 3",
            "readyok",
            "",
        ] {
            assert_eq!(parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn an_engine_cannot_overflow_a_score() {
        let wild = [
            "info depth 1 score cp -2147483648 pv e2e4",
            "info depth 1 score cp 2147483647 pv e2e4",
            "info depth 1 score mate -2147483648 pv e2e4",
            "info depth 1 score mate 2147483647 pv e2e4",
        ];
        let black = "8/8/8/8/8/8/8/K6k b - - 0 1";
        for line in wild {
            let Some(Said::Info { score, .. }) = parse(line) else {
                panic!("{line:?} did not parse");
            };
            let score = white_relative(score, black);
            let _ = (score.label(), score.signed(), score.white_share());
            assert!(score.centipawns().abs() <= MATE + MATE_MAX, "{line:?}");
        }
    }

    #[test]
    fn scores_are_turned_round_for_black() {
        let white = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let black = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        assert_eq!(white_relative(Score::Cp(30), white), Score::Cp(30));
        assert_eq!(white_relative(Score::Cp(30), black), Score::Cp(-30));
        assert_eq!(white_relative(Score::Mate(2), black), Score::Mate(-2));
    }

    #[test]
    fn the_bar_and_its_label_follow_the_score() {
        assert!((Score::Cp(0).white_share() - 0.5).abs() < 1e-6);
        assert!(Score::Cp(300).white_share() > 0.7);
        assert!(Score::Mate(-1).white_share() < 0.01);
        assert!(Score::Mate(3).centipawns() > Score::Mate(5).centipawns());
        assert!(Score::Mate(-3).centipawns() < Score::Mate(-5).centipawns());
        assert_eq!(Score::Cp(134).label(), "1.3");
        assert_eq!(Score::Cp(-1234).label(), "12");
        assert_eq!(Score::Mate(-4).label(), "M4");
        assert_eq!(Score::Cp(-40).signed(), "-0.4");
        assert_eq!(Score::Mate(-3).signed(), "-M3");
        assert_eq!(Score::Mate(2).signed(), "M2");
        assert_eq!(Score::mated(true), Score::Cp(-MATE));
    }
}
