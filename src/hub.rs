//! The main loop: the lobby, the game being played, and everything that
//! connects them to the opponent.
//!
//! None of this knows which game is being played. It pairs players and hands
//! each game's keys, clicks and messages to its [`Play`].

use std::time::Duration;

use anyhow::Result;
use iroh::{Endpoint, EndpointId};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use tui_tui::clipboard;
use tui_tui::games::{Conn, Ctx, Kind, Leave, Play, Seat, Table};
use tui_tui::lobby::{self, Choice, Invited, Lobby, Row};
use tui_tui::net::{self, NetEvent};
use tui_tui::profile::Profile;
use tui_tui::session::{self, Code, Incoming, Invite, Link, Listener, Target};

/// Roughly 60 frames a second while something is moving.
const FRAME: Duration = Duration::from_millis(16);

/// Everything that can wake the main loop.
pub enum Wake {
    Input(Event),
    Heard(Incoming),
    /// A code joined, or a friend invited, for the match with this id.
    Paired(u64, Result<Link, String>),
    Net(u64, NetEvent),
    /// Something working in the background has news to draw.
    Redraw,
}

enum Screen {
    Lobby(Lobby),
    Match(Match),
}

/// One game, from the moment it is chosen in the lobby until it is left.
struct Match {
    /// Tells this match's events from those of matches already left.
    id: u64,
    table: Box<Table<dyn Play>>,
    /// Whether the listener's next [`Incoming::Paired`] is for us.
    by_listener: bool,
    /// Our own dialling out, if that is how this match is being paired.
    dialling: Option<JoinHandle<()>>,
}

impl Drop for Match {
    fn drop(&mut self) {
        if let Some(task) = self.dialling.take() {
            task.abort();
        }
    }
}

enum Next {
    Stay,
    Go(Screen),
    Exit,
}

pub struct Hub {
    profile: Profile,
    endpoint: Endpoint,
    listener: Listener,
    wake: UnboundedSender<Wake>,
    next_match: u64,
    /// A friend's invite, while the lobby asks about it.
    invite: Option<Invite>,
    /// Something for the next lobby to say.
    notice: Option<String>,
    /// The game last picked, for the next lobby to start on.
    game: Kind,
}

impl Hub {
    /// Starts answering whoever dials in, for any game this build can play.
    pub fn new(
        profile: Profile,
        endpoint: Endpoint,
        wake: UnboundedSender<Wake>,
        notice: Option<String>,
    ) -> Self {
        let (heard, mut hearing) = unbounded_channel();
        let listener = Listener::start(endpoint.clone(), &Kind::all_wire(), heard);
        tokio::spawn({
            let wake = wake.clone();
            async move {
                while let Some(incoming) = hearing.recv().await {
                    let _ = wake.send(Wake::Heard(incoming));
                }
            }
        });
        Self {
            profile,
            endpoint,
            listener,
            wake,
            next_match: 0,
            invite: None,
            notice,
            game: Kind::DEFAULT,
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the one match on what woke the loop, which reads best whole"
    )]
    pub async fn run(
        &mut self,
        term: &mut Term,
        woken: &mut UnboundedReceiver<Wake>,
        game: Kind,
        start: Option<Choice>,
    ) -> Result<()> {
        self.game = game;
        let mut screen = match start {
            None => self.lobby(None),
            Some(choice) => {
                let mut lobby = self.new_lobby(None);
                match self.choose(choice, &mut lobby) {
                    Next::Go(screen) => screen,
                    Next::Stay => Screen::Lobby(lobby),
                    Next::Exit => return Ok(()),
                }
            }
        };

        loop {
            let size = term.terminal.size()?;
            let area = Rect::new(0, 0, size.width, size.height);
            match &mut screen {
                Screen::Lobby(lobby) => {
                    term.set_mouse(true);
                    lobby.area = area;
                    term.terminal.draw(|f| lobby::draw_lobby(f, lobby))?;
                }
                Screen::Match(m) => {
                    match m.table.leaving() {
                        Some(Leave::Exit) => return Ok(()),
                        Some(Leave::Lobby) => {
                            let last = m.table.ctx.peer;
                            self.game = m.table.kind();
                            screen = self.lobby(last);
                            continue;
                        }
                        None => {}
                    }
                    term.set_mouse(m.table.wants_mouse());
                    let ctx = &mut m.table.ctx;
                    if let Some(text) = ctx.copy_request.take() {
                        ctx.copied = Some(clipboard::copy(&text));
                    }
                    m.table.set_area(area);
                    term.terminal.draw(|f| m.table.draw(f))?;
                }
            }

            // While something is moving we redraw on a timer; the rest of
            // the time the loop sits idle waiting for something to happen.
            let animating = matches!(&screen, Screen::Match(m) if m.table.is_animating());
            let wake = tokio::select! {
                wake = woken.recv() => match wake {
                    Some(wake) => wake,
                    None => return Ok(()),
                },
                () = tokio::time::sleep(FRAME), if animating => continue,
            };

            let next = match (wake, &mut screen) {
                (Wake::Input(ev), Screen::Lobby(lobby)) => {
                    let choice = match ev {
                        Event::Key(key) => lobby.on_key(key),
                        Event::Mouse(mouse) => lobby.on_mouse(mouse),
                        Event::Paste(text) => {
                            lobby.on_paste(&text);
                            None
                        }
                        _ => None,
                    };
                    match choice {
                        Some(choice) => self.choose(choice, lobby),
                        None => Next::Stay,
                    }
                }
                (Wake::Input(ev), Screen::Match(m)) => {
                    match ev {
                        Event::Key(key) => m.table.on_key(key),
                        Event::Mouse(mouse) => m.table.on_mouse(mouse),
                        _ => {}
                    }
                    Next::Stay
                }
                // The game acts on it, and it is drawn at the top of the loop.
                (Wake::Redraw, Screen::Match(m)) => {
                    m.table.on_wake();
                    Next::Stay
                }
                (Wake::Heard(incoming), screen) => {
                    self.heard(incoming, screen);
                    Next::Stay
                }
                (Wake::Paired(id, result), Screen::Match(m)) if m.id == id => {
                    m.dialling = None;
                    match result {
                        Ok(link) => self.play(m, link),
                        Err(why) => m.table.lost(why),
                    }
                    Next::Stay
                }
                (Wake::Net(id, ev), Screen::Match(m)) if m.id == id => {
                    m.table.on_net(ev);
                    Next::Stay
                }
                // A redraw with no game to act on it, or news left over from
                // a match that has been left.
                _ => Next::Stay,
            };
            match next {
                Next::Stay => {}
                Next::Go(next) => screen = next,
                Next::Exit => return Ok(()),
            }
        }
    }

    fn lobby(&mut self, last: Option<EndpointId>) -> Screen {
        Screen::Lobby(self.new_lobby(last))
    }

    /// A fresh lobby on the last game played, with the last opponent picked
    /// out for a rematch.
    fn new_lobby(&mut self, last: Option<EndpointId>) -> Lobby {
        self.listener.idle();
        let mut lobby = Lobby::new();
        lobby.game = Kind::ALL.iter().position(|&k| k == self.game).unwrap_or(0);
        lobby.name.clone_from(&self.profile.name);
        lobby.guest = self.profile.is_guest();
        lobby.set_friends(self.profile.contacts.clone());
        lobby.notice = self.notice.take();
        if let Some(i) = last.and_then(|id| self.profile.contacts.iter().position(|c| c.id == id)) {
            let rows = lobby.rows();
            if let Some(row) = rows.iter().position(|&r| r == Row::Friend(i)) {
                lobby.selected = row;
            }
        }
        lobby
    }

    fn choose(&mut self, choice: Choice, lobby: &mut Lobby) -> Next {
        // Starting anything means we cannot also answer a friend.
        let starts_match = matches!(
            choice,
            Choice::Host | Choice::Join(_) | Choice::Local | Choice::Challenge(_)
        );
        if starts_match && let Some(invite) = self.invite.take() {
            invite.refuse("busy");
        }

        let game = lobby.game();
        let m = match choice {
            Choice::Quit => return Next::Exit,
            Choice::Local => {
                self.listener.busy();
                self.new_match(game.start(Seat::Local, Ctx::local()), false)
            }
            Choice::Host => self.host(game),
            Choice::Join(code) => self.join(code, game),
            Choice::Challenge(id) => self.challenge(id, game),
            Choice::AcceptInvite => {
                let Some(invite) = self.invite.take() else {
                    return Next::Stay;
                };
                let Some(game) = Kind::from_wire(invite.game) else {
                    invite.refuse("unsupported-game");
                    return Next::Stay;
                };
                self.listener.busy();
                invite.accept(&self.profile.name);
                // Whoever is invited goes first, as whoever hosts does.
                let ctx = Ctx::new(Conn::Dialling, None);
                self.new_match(game.start(Seat::Host, ctx), true)
            }
            Choice::DeclineInvite => {
                if let Some(invite) = self.invite.take() {
                    invite.refuse("declined");
                }
                return Next::Stay;
            }
            Choice::Rename(name) => {
                match self.profile.set_name(&name) {
                    Ok(()) => lobby.name.clone_from(&self.profile.name),
                    Err(e) => lobby.notice = Some(format!("{e:#}")),
                }
                return Next::Stay;
            }
            Choice::Forget(id) => {
                let name = self.profile.contact(id).map(|c| c.name.clone());
                if let Err(e) = self.profile.forget(id) {
                    lobby.notice = Some(format!("{e:#}"));
                } else if let Some(name) = name {
                    lobby.notice = Some(format!("forgot {name}"));
                }
                lobby.set_friends(self.profile.contacts.clone());
                return Next::Stay;
            }
        };
        Next::Go(Screen::Match(m))
    }

    /// For a game's background work to have the screen drawn again.
    fn redraw(&self) -> tui_tui::games::Waker {
        let wake = self.wake.clone();
        std::sync::Arc::new(move || {
            let _ = wake.send(Wake::Redraw);
        })
    }

    fn new_match(&mut self, mut table: Box<Table<dyn Play>>, by_listener: bool) -> Match {
        table.set_waker(self.redraw());
        self.next_match += 1;
        Match {
            id: self.next_match,
            table,
            by_listener,
            dialling: None,
        }
    }

    fn host(&mut self, game: Kind) -> Match {
        let code = Code::generate();
        self.listener
            .host(code, game.wire(), &self.profile.name, true);
        let mut ctx = Ctx::new(Conn::Publishing, Some(code));
        // Hosting is for sending the code to someone, so have it ready.
        ctx.copy_share();
        let m = game.start(Seat::Host, ctx);
        self.new_match(m, true)
    }

    /// Joins whatever the code's host is playing. Until that is known, the
    /// game picked in the lobby stands in.
    fn join(&mut self, code: Code, game: Kind) -> Match {
        self.listener.busy();
        let ctx = Ctx::new(Conn::LookingUp, None);
        let mut m = self.new_match(game.start(Seat::Guest, ctx), false);
        let (id, endpoint, wake, name) = (
            m.id,
            self.endpoint.clone(),
            self.wake.clone(),
            self.profile.name.clone(),
        );
        m.dialling = Some(tokio::spawn(async move {
            let progress = {
                let wake = wake.clone();
                move |p| {
                    let _ = wake.send(Wake::Net(id, NetEvent::Progress(p)));
                }
            };
            let games = Kind::all_wire();
            let result =
                session::join(&endpoint, code, &games, Target::Lookup, &name, progress).await;
            let result = result.map_err(|e| session::explain(&e, "your opponent"));
            let _ = wake.send(Wake::Paired(id, result));
        }));
        m
    }

    fn challenge(&mut self, friend: EndpointId, game: Kind) -> Match {
        self.listener.busy();
        let their_name = self
            .profile
            .contact(friend)
            .map_or_else(|| friend.fmt_short().to_string(), |c| c.name.clone());
        let ctx = Ctx::new(Conn::Inviting(their_name.clone()), None);
        let mut m = self.new_match(game.start(Seat::Guest, ctx), false);
        let (id, endpoint, wake, name) = (
            m.id,
            self.endpoint.clone(),
            self.wake.clone(),
            self.profile.name.clone(),
        );
        m.dialling = Some(tokio::spawn(async move {
            let result = session::invite(&endpoint, friend, game.wire(), &name).await;
            let result = result.map_err(|e| session::explain(&e, &their_name));
            let _ = wake.send(Wake::Paired(id, result));
        }));
        m
    }

    fn heard(&mut self, incoming: Incoming, screen: &mut Screen) {
        let waiting = match screen {
            Screen::Match(m) if m.by_listener && !m.table.ctx.is_networked() => Some(m),
            _ => None,
        };
        match incoming {
            Incoming::Invite(invite) => {
                let Some(friend) = self.profile.contact(invite.peer) else {
                    invite.refuse("unknown");
                    return;
                };
                let Some(game) = Kind::from_wire(invite.game) else {
                    invite.refuse("unsupported-game");
                    return;
                };
                match screen {
                    Screen::Lobby(lobby) if self.invite.is_none() => {
                        lobby.invite = Some(Invited {
                            name: friend.name.clone(),
                            game,
                        });
                        self.invite = Some(invite);
                    }
                    _ => invite.refuse("busy"),
                }
            }
            Incoming::InviteGone(peer) => {
                if self.invite.as_ref().is_some_and(|i| i.peer == peer) {
                    self.invite = None;
                    if let Screen::Lobby(lobby) = screen {
                        let name = lobby.invite.take().map(|i| i.name).unwrap_or_default();
                        lobby.notice = Some(format!("{name}'s invite ran out"));
                    }
                }
            }
            Incoming::Paired(link) => match waiting {
                Some(m) => self.play(m, link),
                // Nobody here is waiting for it any more; dropping it hangs up.
                None => drop(link),
            },
            Incoming::Progress(p) => {
                if let Some(m) = waiting {
                    m.table.on_net(NetEvent::Progress(p));
                }
            }
            Incoming::Closed(why) => {
                if let Some(m) = waiting {
                    m.table.lost(why);
                }
            }
        }
    }

    /// The opponent is in: remember them, and start playing.
    fn play(&mut self, m: &mut Match, link: Link) {
        // Joining by code finds out only now what the host is playing.
        if let Some(kind) = Kind::from_wire(link.game)
            && kind != m.table.kind()
        {
            m.table = kind.start(Seat::Guest, Ctx::new(Conn::Dialling, None));
            m.table.set_waker(self.redraw());
        }
        if let Err(e) = self.profile.played(link.peer, &link.peer_name) {
            self.notice = Some(format!("could not save your friends: {e:#}"));
        }
        let name = self
            .profile
            .contact(link.peer)
            .map_or_else(|| link.peer_name.clone(), |c| c.name.clone());

        let (events, mut heard) = unbounded_channel();
        let (id, wake) = (m.id, self.wake.clone());
        tokio::spawn(async move {
            while let Some(ev) = heard.recv().await {
                let _ = wake.send(Wake::Net(id, ev));
            }
        });
        m.table.attach(net::play(link, events), &name);
    }
}

/// The terminal, and the modes we have switched on in it that need switching
/// off again on the way out.
pub struct Term {
    terminal: DefaultTerminal,
    capturing: bool,
}

impl Term {
    pub fn new() -> Self {
        let terminal = ratatui::init();
        // A pasted code then arrives as one event rather than as keystrokes,
        // some of which the lobby would take as commands.
        let _ = execute!(std::io::stdout(), EnableBracketedPaste);
        Self {
            terminal,
            capturing: false,
        }
    }

    fn set_mouse(&mut self, on: bool) {
        if on == self.capturing {
            return;
        }
        self.capturing = on;
        let _ = if on {
            execute!(std::io::stdout(), EnableMouseCapture)
        } else {
            execute!(std::io::stdout(), DisableMouseCapture)
        };
    }

    pub fn restore(mut self) {
        self.set_mouse(false);
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}
