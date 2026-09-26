//! Pairing by code, then chess over the link: both halves of a real
//! connection in one process.
//!
//! These dial the host's address directly, so they do not depend on the DHT;
//! the code still has to match for the handshake to let anyone in.

use std::time::Duration;

use iroh::{Endpoint, SecretKey};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tui_tui::games::chess::GAME as CHESS;
use tui_tui::net::{self, NetEvent};
use tui_tui::session::{self, Code, Incoming, Link, Listener, MAX_WRONG_CODES, Progress, Target};

/// Wait for the next thing on a channel, failing loudly rather than hanging.
async fn next<T>(rx: &mut UnboundedReceiver<T>, what: &str) -> T {
    tokio::time::timeout(Duration::from_secs(30), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
        .unwrap_or_else(|| panic!("channel closed waiting for {what}"))
}

struct Player {
    endpoint: Endpoint,
    listener: Listener,
    heard: UnboundedReceiver<Incoming>,
}

async fn player() -> Player {
    let endpoint = session::bind(SecretKey::generate()).await.unwrap();
    endpoint.online().await;
    let (tx, heard) = unbounded_channel();
    let listener = Listener::start(endpoint.clone(), &[CHESS], tx);
    Player {
        endpoint,
        listener,
        heard,
    }
}

/// A host waiting for `code`, not on the DHT.
async fn host(code: Code) -> Player {
    let host = player().await;
    host.listener.host(code, CHESS, "alice", false);
    host
}

async fn dial(host: &Player, guest: &Player, code: Code) -> anyhow::Result<Link> {
    let at = Target::Addr(host.endpoint.addr());
    session::join(&guest.endpoint, code, &[CHESS], at, "bob", |_| {}).await
}

async fn paired(host: &mut Player) -> Link {
    match next(&mut host.heard, "the host's link").await {
        Incoming::Paired(link) => link,
        other => panic!("expected Paired, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_peers_pair_by_code_and_play() {
    let code = Code::generate();
    let mut alice = host(code).await;
    let bob = player().await;

    let bobs = dial(&alice, &bob, code).await.expect("bob gets in");
    let alices = paired(&mut alice).await;
    assert_eq!(alices.peer, bob.endpoint.id());
    assert_eq!(alices.peer_name, "bob");
    assert_eq!(bobs.peer, alice.endpoint.id());
    assert_eq!(bobs.peer_name, "alice");

    let (tx, mut alice_rx) = unbounded_channel();
    let white = net::play(alices, tx);
    let (tx, mut bob_rx) = unbounded_channel();
    let black = net::play(bobs, tx);

    white.send("move e2e4".into());
    match next(&mut bob_rx, "white's move").await {
        NetEvent::Line(line) => assert_eq!(line, "move e2e4"),
        other => panic!("expected a line, got {other:?}"),
    }
    black.send("move e7e5".into());
    match next(&mut alice_rx, "black's reply").await {
        NetEvent::Line(line) => assert_eq!(line, "move e7e5"),
        other => panic!("expected a line, got {other:?}"),
    }
    black.send("resign".into());
    match next(&mut alice_rx, "resignation").await {
        NetEvent::Line(line) => assert_eq!(line, "resign"),
        other => panic!("expected a line, got {other:?}"),
    }

    // Leaving says goodbye, so the other side hears why rather than just
    // losing the connection.
    drop(black);
    match next(&mut alice_rx, "bob leaving").await {
        NetEvent::Disconnected(why) => assert_eq!(why, "your opponent left"),
        other => panic!("expected Disconnected, got {other:?}"),
    }
}

/// A wrong code is turned away, and the host keeps waiting for the right one.
#[tokio::test(flavor = "multi_thread")]
async fn wrong_code_is_refused() {
    let code = Code::generate();
    let mut alice = host(code).await;
    let mallory = player().await;

    let refused = dial(&alice, &mallory, Code::generate()).await;
    let why = format!("{:#}", refused.expect_err("a wrong code gets nowhere"));
    assert!(why.contains("did not match"), "{why}");
    assert!(matches!(
        next(&mut alice.heard, "alice notices").await,
        Incoming::Progress(Progress::WrongCode { attempts: 1 })
    ));

    let bob = player().await;
    // Held on to: dropping it would hang up before bob's last word is sent.
    let _bobs = dial(&alice, &bob, code).await.expect("bob still gets in");
    assert_eq!(paired(&mut alice).await.peer, bob.endpoint.id());
}

/// Each wrong code is a guess, so the host only allows a few.
#[tokio::test(flavor = "multi_thread")]
async fn host_stops_after_too_many_wrong_codes() {
    let code = Code::generate();
    let mut alice = host(code).await;
    let mallory = player().await;

    for attempt in 1..=MAX_WRONG_CODES {
        assert!(dial(&alice, &mallory, Code::generate()).await.is_err());
        match next(&mut alice.heard, "alice notices").await {
            Incoming::Progress(Progress::WrongCode { attempts }) => assert_eq!(attempts, attempt),
            other => panic!("expected WrongCode, got {other:?}"),
        }
    }
    match next(&mut alice.heard, "alice gives up").await {
        Incoming::Closed(why) => assert!(why.contains("too many wrong codes"), "{why}"),
        other => panic!("expected Closed, got {other:?}"),
    }
    // Even the right code is no good now.
    let bob = player().await;
    let late = dial(&alice, &bob, code).await.expect_err("hosting is over");
    assert!(session::explain(&late, "alice").contains("no longer open"));
}

/// Once the host has gone back to the lobby, its code leads nowhere.
#[tokio::test(flavor = "multi_thread")]
async fn a_code_stops_working_when_the_host_moves_on() {
    let code = Code::generate();
    let alice = host(code).await;
    alice.listener.idle();

    let bob = player().await;
    let err = dial(&alice, &bob, code)
        .await
        .expect_err("nobody is hosting");
    assert!(session::explain(&err, "alice").contains("no longer open"));
}

/// The whole path a player takes: publish, look the code up, pair. Needs the
/// internet; run it with `cargo test -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the internet"]
async fn pair_by_code_over_the_dht() {
    let code = Code::generate();
    let mut alice = player().await;
    alice.listener.host(code, CHESS, "alice", true);
    match next(&mut alice.heard, "code published").await {
        Incoming::Progress(Progress::Listed) => {}
        other => panic!("expected Listed, got {other:?}"),
    }

    let bob = player().await;
    let link = session::join(&bob.endpoint, code, &[CHESS], Target::Lookup, "bob", |_| {})
        .await
        .expect("bob finds alice by code");
    assert_eq!(link.peer, alice.endpoint.id());
    assert_eq!(paired(&mut alice).await.peer_name, "bob");
}
