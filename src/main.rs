//! tuitui — terminal games over a direct peer-to-peer connection.
//!
//! ```text
//! tuitui                 the lobby: pick a game, host, join, challenge a friend
//! tuitui play [game]     the same, on the game named
//! tuitui local [game]    two players, one keyboard (or one, for wordle)
//! tuitui host [game]     wait for an opponent, and go first
//! tuitui join <code>     join with an opponent's code
//! ```
//!
//! Without a game named, the first there is; the lobby starts on whichever is.

mod hub;

use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use hub::{Hub, Term, Wake};
use tui_tui::games::Kind;
use tui_tui::lobby::Choice;
use tui_tui::profile::Profile;
use tui_tui::session;

/// The longest quitting waits for connections to close cleanly.
const SHUTDOWN: Duration = Duration::from_secs(2);

const USAGE: &str = "\
tuitui — terminal games over iroh

usage:
  tuitui                  open the lobby: pick a game, host, join, or
                          challenge a friend
  tuitui play [game]      the same, starting on the game named
  tuitui local [game]     play locally: two players on one keyboard,
                          or on your own for a game like wordle
  tuitui host [game]      host a game and get a code to share
  tuitui join <code>      join a game with the code your opponent sent,
                          e.g. tuitui join 42-tiger-marble-ocean

games: {games}

Your profile (identity, name, friends) is kept in tui-tui under your
config directory, or in $TUI_TUI_HOME if that is set.
";

/// The usage text, with the games this build can play filled in.
fn usage() -> String {
    // The first is the one played when none is named.
    let games: Vec<String> = Kind::ALL
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let name = k.name().to_lowercase();
            if i == 0 {
                format!("{name} (default)")
            } else {
                name
            }
        })
        .collect();
    USAGE.replace("{games}", &games.join(", "))
}

/// What the command line asked for: which game the lobby starts on, and
/// whatever is to happen without waiting for the lobby.
#[derive(Debug, PartialEq)]
struct Start {
    game: Kind,
    choice: Option<Choice>,
}

/// What to do instead of playing: say something and stop.
#[derive(Debug, PartialEq)]
enum Say {
    Usage,
    Version,
    /// A complaint, then the usage, on the way to exit code 2.
    Wrong(String),
}

/// Reads the command line. Settled before the terminal is taken over, so a
/// bad code or a typo is reported on a normal screen.
fn parse(args: &[&str]) -> Result<Start, Say> {
    let (verb, rest) = args.split_first().map_or(("", &[][..]), |(v, r)| (*v, r));
    // `tuitui chess` is shorthand for `tuitui play chess`.
    let (verb, rest) = match Kind::from_name(verb) {
        Some(_) => ("play", args),
        None => (verb, rest),
    };

    // Every verb but join takes a game, or none and gets the first.
    let named = || match rest {
        [] => Ok(Kind::DEFAULT),
        [name] => {
            Kind::from_name(name).ok_or_else(|| Say::Wrong(format!("no game called {name:?}")))
        }
        [_, extra, ..] => Err(Say::Wrong(format!("unexpected {extra:?}"))),
    };
    let (game, choice) = match verb {
        "" | "play" => (named()?, None),
        "local" => (named()?, Some(Choice::Local)),
        "host" => (named()?, Some(Choice::Host)),
        // The host decides the game, so joining never names one. Spaces are
        // fine as well as dashes, as in `join 42 tiger marble ocean`.
        "join" if !rest.is_empty() => {
            let code = rest
                .join("-")
                .parse()
                .map_err(|e| Say::Wrong(format!("{e}")))?;
            (Kind::DEFAULT, Some(Choice::Join(code)))
        }
        "-h" | "--help" | "help" => return Err(Say::Usage),
        "-V" | "--version" | "version" => return Err(Say::Version),
        "join" => return Err(Say::Wrong("join needs a code".into())),
        other => return Err(Say::Wrong(format!("unexpected {other:?}"))),
    };
    Ok(Start { game, choice })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let Start { game, choice } = match parse(&args) {
        Ok(start) => start,
        Err(Say::Usage) => {
            print!("{}", usage());
            return Ok(());
        }
        Err(Say::Version) => {
            println!("tuitui {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Err(Say::Wrong(why)) => {
            eprintln!("tuitui: {why}\n");
            eprint!("{}", usage());
            std::process::exit(2);
        }
    };

    let (profile, notice) = match Profile::load() {
        Ok(profile) => (profile, None),
        Err(e) => (Profile::guest(), Some(format!("playing as a guest: {e:#}"))),
    };
    let endpoint = session::bind(profile.secret.clone()).await?;

    let (wake, mut woken) = unbounded_channel();
    spawn_input(wake.clone());
    let mut hub = Hub::new(profile, endpoint.clone(), wake, notice);
    let mut term = Term::new();
    let result = hub.run(&mut term, &mut woken, game, choice).await;
    term.restore();
    // Dropping the hub ends any game, which says goodbye to the peer on its
    // way out; closing the endpoint gives that a moment to be sent.
    drop(hub);
    let _ = tokio::time::timeout(SHUTDOWN, endpoint.close()).await;
    result
}

/// crossterm's reader is blocking, so it lives on its own thread and feeds the
/// async loop through the same channel as everything else.
fn spawn_input(wake: UnboundedSender<Wake>) {
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if wake.send(Wake::Input(ev)).is_err() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(args: &[&str]) -> Start {
        parse(args).expect("should parse")
    }

    #[test]
    fn the_bare_command_opens_the_lobby() {
        assert_eq!(start(&[]).choice, None);
        assert_eq!(start(&["play"]).choice, None);
        assert_eq!(start(&[]).game, Kind::DEFAULT);
    }

    #[test]
    fn a_game_can_be_named_after_the_verb_or_on_its_own() {
        for args in [&["play", "chess"][..], &["chess"][..]] {
            assert_eq!(
                start(args),
                Start {
                    game: tui_tui::games::chess::KIND,
                    choice: None
                },
                "{args:?}"
            );
        }
        assert_eq!(start(&["local", "chess"]).choice, Some(Choice::Local));
        assert_eq!(start(&["host", "chess"]).game, tui_tui::games::chess::KIND);
    }

    #[test]
    fn the_verbs_start_what_they_say() {
        assert_eq!(start(&["local"]).choice, Some(Choice::Local));
        assert_eq!(start(&["host"]).choice, Some(Choice::Host));
        let code = "42-tiger-marble-ocean";
        assert_eq!(
            start(&["join", code]).choice,
            Some(Choice::Join(code.parse().unwrap()))
        );
        // A code pasted with spaces arrives as several arguments.
        assert_eq!(
            start(&["join", "42", "tiger", "marble", "ocean"]).choice,
            Some(Choice::Join(code.parse().unwrap()))
        );
    }

    #[test]
    fn help_and_version_say_so() {
        for args in [&["-h"][..], &["--help"][..], &["help"][..]] {
            assert_eq!(parse(args), Err(Say::Usage), "{args:?}");
        }
        for args in [&["-V"][..], &["--version"][..], &["version"][..]] {
            assert_eq!(parse(args), Err(Say::Version), "{args:?}");
        }
    }

    #[test]
    fn anything_else_is_a_complaint_rather_than_a_guess() {
        for args in [
            &["wat"][..],
            &["local", "draughts"][..],
            &["host", "chess", "extra"][..],
            &["join"][..],
            &["join", "nonsense"][..],
        ] {
            assert!(matches!(parse(args), Err(Say::Wrong(_))), "{args:?}");
        }
    }
}
