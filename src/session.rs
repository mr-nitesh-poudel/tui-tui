//! Getting two players connected, whatever they are going to play.
//!
//! There are two ways in. Strangers pair with a short [`Code`]: the host
//! publishes its address under it on the DHT ([`rendezvous`]), the joiner
//! looks it up and dials, and both prove they hold the same code. Friends,
//! whose endpoint ids were saved the first time, skip all that: one [invites]
//! the other directly, and iroh proves who each of them is.
//!
//! Each player has one endpoint for the whole run. The [`Listener`] answers
//! whatever dials in, according to what the player is doing: in the lobby it
//! passes friends' invites on, while hosting it pairs with a code, and during
//! a game it turns everyone away.
//!
//! Either way the caller ends up with an authenticated [`Link`] to speak its
//! own game's protocol over.
//!
//! [invites]: invite

pub mod code;
pub mod handshake;
pub mod lines;
pub mod name;
pub mod rendezvous;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::{SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use code::{Code, CodeError};
pub use handshake::{Declined, Reader, Refused, WrongCode};

/// One protocol for every game: which one is being played is settled inside
/// the handshake rather than by ALPN.
pub const ALPN: &[u8] = b"tui-tui/session/1";

/// How long one attempt at the handshake may take before it is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

/// How long to wait for the endpoint to reach a relay, without which there is
/// nothing worth publishing.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to keep trying to reach a friend before calling them offline.
const REACH_TIMEOUT: Duration = Duration::from_secs(20);

/// How long an invite waits for its answer.
pub const INVITE_TIMEOUT: Duration = Duration::from_mins(1);

/// Wrong codes the host puts up with before it stops listening. Each is one
/// guess at the code, so this bounds the odds of an attacker getting in.
pub const MAX_WRONG_CODES: u32 = 3;

const PUBLISH_ATTEMPTS: u32 = 3;

/// A game and the version of its wire protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Game {
    pub name: &'static str,
    pub version: u32,
}

/// How pairing by code is going, for the UI to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The host's code is on the DHT and can be looked up.
    Listed,
    /// The joiner has found the host and is dialling it.
    Found,
    /// Someone dialled the host with the wrong code.
    WrongCode { attempts: u32 },
    /// Someone got in but could not play what the host is playing.
    Declined,
}

/// A connected, authenticated peer, ready for the game's own protocol.
pub struct Link {
    pub peer: EndpointId,
    /// What the peer calls itself. Only a label: `peer` is who it really is.
    pub peer_name: String,
    pub game: Game,
    pub send: SendStream,
    pub lines: Reader,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("peer", &self.peer)
            .field("peer_name", &self.peer_name)
            .field("game", &self.game)
            .finish_non_exhaustive()
    }
}

/// Where the joiner finds the host.
pub enum Target {
    /// Look the code up on the DHT.
    Lookup,
    /// Dial this address directly; the code is still needed for the handshake.
    Addr(EndpointAddr),
}

/// The friend could not be reached at all.
#[derive(Debug)]
pub struct Unreachable;

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("could not reach them")
    }
}

impl std::error::Error for Unreachable {}

/// An iroh endpoint with `secret` as its identity, speaking our ALPN.
///
/// # Errors
///
/// If the endpoint cannot be bound.
pub async fn bind(secret: SecretKey) -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("binding iroh endpoint")
}

/// Find the host behind `code` and pair with it, accepting any of `games`.
///
/// # Errors
///
/// If the code leads nowhere, the host cannot be reached, or the handshake
/// fails: a wrong code, a game this build does not have, or a refusal.
pub async fn join(
    endpoint: &Endpoint,
    code: Code,
    games: &[Game],
    target: Target,
    name: &str,
    progress: impl Fn(Progress),
) -> Result<Link> {
    let addr = match target {
        Target::Lookup => rendezvous::resolve(code).await?,
        Target::Addr(addr) => addr,
    };
    progress(Progress::Found);

    let host = addr.id;
    let conn = endpoint
        .connect(addr, ALPN)
        .await
        .context("found the game, but could not reach your opponent")?;
    let (mut send, recv) = conn.open_bi().await.context("opening stream")?;
    let mut lines = Reader::new(recv);
    let (game, peer_name) = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        handshake::join_code(
            &mut send,
            &mut lines,
            code,
            games,
            name,
            (host, endpoint.id()),
        ),
    )
    .await
    .context("your opponent stopped answering")??;
    Ok(Link {
        peer: host,
        peer_name,
        game,
        send,
        lines,
    })
}

/// Ask a friend to play `game`, and wait for their answer. `friend` is
/// normally just their endpoint id, which iroh finds the rest of.
///
/// # Errors
///
/// If the friend cannot be reached, does not answer in time, or says no.
pub async fn invite(
    endpoint: &Endpoint,
    friend: impl Into<EndpointAddr>,
    game: Game,
    name: &str,
) -> Result<Link> {
    let addr = friend.into();
    let peer = addr.id;
    let conn = tokio::time::timeout(REACH_TIMEOUT, endpoint.connect(addr, ALPN))
        .await
        .map_err(|_| Unreachable)?
        .map_err(|_| Unreachable)?;
    let (mut send, recv) = conn.open_bi().await.map_err(|_| Unreachable)?;
    let mut lines = Reader::new(recv);
    let peer_name = tokio::time::timeout(
        INVITE_TIMEOUT + HANDSHAKE_TIMEOUT,
        handshake::invite(&mut send, &mut lines, game, name),
    )
    .await
    .map_err(|_| Refused("timeout".into()))??;
    Ok(Link {
        peer,
        peer_name,
        game,
        send,
        lines,
    })
}

/// A pairing error, told to the player. `who` is how to refer to the other
/// side: a friend's name, or "your opponent".
#[must_use]
pub fn explain(err: &anyhow::Error, who: &str) -> String {
    if err.is::<Unreachable>() {
        return format!("{who} is not online");
    }
    if let Some(Refused(why)) = err.downcast_ref() {
        return match why.as_str() {
            "declined" => format!("{who} said no"),
            "busy" => format!("{who} is busy right now"),
            "unknown" => format!("{who} no longer has you as a friend"),
            "timeout" => format!("{who} did not answer"),
            "not-hosting" => "that game is no longer open — ask for a new code".into(),
            other => format!("{who} refused ({other})"),
        };
    }
    format!("{err:#}")
}

/// What the listening side has to report.
#[derive(Debug)]
pub enum Incoming {
    Progress(Progress),
    /// Hosting by code ended without an opponent.
    Closed(String),
    /// Someone is in, by code or by an accepted invite.
    Paired(Link),
    /// A friend wants to play. Answer it, or drop it to say no.
    Invite(Invite),
    /// The invite from this peer was withdrawn or timed out unanswered.
    InviteGone(EndpointId),
}

#[derive(Debug)]
pub struct Invite {
    pub peer: EndpointId,
    pub name: String,
    pub game: Game,
    reply: oneshot::Sender<Reply>,
}

#[derive(Debug)]
enum Reply {
    Accept(String),
    Refuse(&'static str),
}

impl Invite {
    /// Say yes; the link then arrives as [`Incoming::Paired`].
    pub fn accept(self, name: &str) {
        let _ = self.reply.send(Reply::Accept(name.to_string()));
    }

    /// Say no. `why` is one of the reasons [`explain`] knows: `declined`,
    /// `busy`, `unknown`.
    pub fn refuse(self, why: &'static str) {
        let _ = self.reply.send(Reply::Refuse(why));
    }
}

#[derive(Clone)]
pub struct Listener {
    inner: Arc<Inner>,
}

struct Inner {
    endpoint: Endpoint,
    games: Vec<Game>,
    mode: Mutex<Mode>,
    publisher: Mutex<Option<JoinHandle<()>>>,
    events: UnboundedSender<Incoming>,
}

enum Mode {
    /// In the lobby: invites are passed on.
    Idle,
    /// Waiting for whoever has the code.
    Hosting(Hosting),
    /// Playing, or otherwise not to be disturbed.
    Busy,
}

#[derive(Clone)]
struct Hosting {
    code: Code,
    game: Game,
    name: String,
    wrong: u32,
}

impl Listener {
    /// Answer everything that dials `endpoint` from now on. It starts busy.
    #[must_use]
    pub fn start(endpoint: Endpoint, games: &[Game], events: UnboundedSender<Incoming>) -> Self {
        let inner = Arc::new(Inner {
            endpoint: endpoint.clone(),
            games: games.to_vec(),
            mode: Mutex::new(Mode::Busy),
            publisher: Mutex::new(None),
            events,
        });
        tokio::spawn({
            let inner = inner.clone();
            async move {
                while let Some(incoming) = endpoint.accept().await {
                    let inner = inner.clone();
                    tokio::spawn(async move {
                        let _ = inner.answer(incoming).await;
                    });
                }
            }
        });
        Self { inner }
    }

    /// Back in the lobby: friends may invite.
    pub fn idle(&self) {
        self.inner.set(Mode::Idle);
    }

    /// In a game: nobody else gets in.
    pub fn busy(&self) {
        self.inner.set(Mode::Busy);
    }

    /// Wait for whoever has `code`, putting it on the DHT first if `publish`.
    pub fn host(&self, code: Code, game: Game, name: &str, publish: bool) {
        self.inner.set(Mode::Hosting(Hosting {
            code,
            game,
            name: name.to_string(),
            wrong: 0,
        }));
        if !publish {
            return;
        }
        let inner = self.inner.clone();
        let publisher = tokio::spawn(async move {
            let result = async {
                tokio::time::timeout(ONLINE_TIMEOUT, inner.endpoint.online())
                    .await
                    .context("could not get online — check your connection")?;
                publish_with_retries(code, &inner.endpoint.addr()).await
            }
            .await;
            match result {
                Ok(()) => inner.emit(Incoming::Progress(Progress::Listed)),
                Err(e) => {
                    *lock(&inner.mode) = Mode::Busy;
                    inner.emit(Incoming::Closed(format!("{e:#}")));
                }
            }
        });
        *lock(&self.inner.publisher) = Some(publisher);
    }
}

/// Takes a lock, poisoned or not. A panic while one is held would otherwise
/// take every later lock with it, and what these guard is a mode and a task
/// handle: there is no half-updated state to protect anyone from.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    fn set(&self, mode: Mode) {
        if let Some(publisher) = lock(&self.publisher).take() {
            publisher.abort();
        }
        *lock(&self.mode) = mode;
    }

    fn emit(&self, event: Incoming) {
        let _ = self.events.send(event);
    }

    fn hosting(&self) -> Option<Hosting> {
        match &*lock(&self.mode) {
            Mode::Hosting(h) => Some(h.clone()),
            _ => None,
        }
    }

    async fn answer(&self, incoming: iroh::endpoint::Incoming) -> Result<()> {
        let conn = incoming.await?;
        let peer = conn.remote_id();
        let (mut send, recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, conn.accept_bi()).await??;
        let mut lines = Reader::new(recv);
        let opening = tokio::time::timeout(HANDSHAKE_TIMEOUT, lines.next_line())
            .await??
            .context("peer hung up")?;
        let (verb, rest) = opening.split_once(' ').unwrap_or((&opening, ""));
        match verb {
            "pake" => self.pair_by_code(peer, send, lines, rest).await,
            "invite" => self.take_invite(peer, send, lines).await,
            _ => {
                handshake::refuse(&mut send, "unknown-opening").await;
                Ok(())
            }
        }
    }

    async fn pair_by_code(
        &self,
        peer: EndpointId,
        mut send: SendStream,
        mut lines: Reader,
        pake: &str,
    ) -> Result<()> {
        let Some(hosting) = self.hosting() else {
            handshake::refuse(&mut send, "not-hosting").await;
            return Ok(());
        };
        let ids = (self.endpoint.id(), peer);
        let shake = handshake::host_code(
            &mut send,
            &mut lines,
            pake,
            hosting.code,
            hosting.game,
            &hosting.name,
            ids,
        );
        let result = tokio::time::timeout(HANDSHAKE_TIMEOUT, shake).await;

        let mut mode = lock(&self.mode);
        // Only count against, or hand over, the hosting this attempt was for:
        // the player may have moved on while it was in flight.
        let Mode::Hosting(current) = &mut *mode else {
            return Ok(());
        };
        if current.code != hosting.code {
            return Ok(());
        }
        match result {
            Ok(Ok(peer_name)) => {
                *mode = Mode::Busy;
                self.emit(Incoming::Paired(Link {
                    peer,
                    peer_name,
                    game: hosting.game,
                    send,
                    lines,
                }));
            }
            Ok(Err(e)) if e.is::<WrongCode>() => {
                current.wrong += 1;
                let attempts = current.wrong;
                self.emit(Incoming::Progress(Progress::WrongCode { attempts }));
                if attempts >= MAX_WRONG_CODES {
                    *mode = Mode::Busy;
                    self.emit(Incoming::Closed(
                        "too many wrong codes — start a new game for a fresh one".into(),
                    ));
                }
            }
            Ok(Err(e)) if e.is::<Declined>() => self.emit(Incoming::Progress(Progress::Declined)),
            // A dropped or stalled attempt does not end the wait: the real
            // opponent may still be on the way.
            _ => {}
        }
        Ok(())
    }

    async fn take_invite(
        &self,
        peer: EndpointId,
        mut send: SendStream,
        mut lines: Reader,
    ) -> Result<()> {
        let (name, game) = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            handshake::read_invite(&mut lines, &self.games),
        )
        .await??;
        let Some(game) = game else {
            handshake::refuse(&mut send, "unsupported-game").await;
            return Ok(());
        };
        if !matches!(*lock(&self.mode), Mode::Idle) {
            handshake::refuse(&mut send, "busy").await;
            return Ok(());
        }

        let (reply, answer) = oneshot::channel();
        self.emit(Incoming::Invite(Invite {
            peer,
            name: name.clone(),
            game,
            reply,
        }));
        let answer = tokio::select! {
            answer = answer => answer.ok(),
            // The guest has nothing more to say until it hears back, so any
            // line, or the stream ending, means it gave up.
            _ = lines.next_line() => {
                self.emit(Incoming::InviteGone(peer));
                return Ok(());
            }
            () = tokio::time::sleep(INVITE_TIMEOUT) => {
                self.emit(Incoming::InviteGone(peer));
                handshake::refuse(&mut send, "timeout").await;
                return Ok(());
            }
        };
        match answer {
            Some(Reply::Accept(my_name)) => {
                handshake::accept_invite(&mut send, &my_name).await?;
                self.emit(Incoming::Paired(Link {
                    peer,
                    peer_name: name,
                    game,
                    send,
                    lines,
                }));
            }
            Some(Reply::Refuse(why)) => handshake::refuse(&mut send, why).await,
            None => handshake::refuse(&mut send, "declined").await,
        }
        Ok(())
    }
}

async fn publish_with_retries(code: Code, addr: &EndpointAddr) -> Result<()> {
    let mut attempt = 1;
    loop {
        match rendezvous::publish(code, addr).await {
            Ok(()) => return Ok(()),
            Err(e) if attempt >= PUBLISH_ATTEMPTS => return Err(e),
            Err(_) => {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
