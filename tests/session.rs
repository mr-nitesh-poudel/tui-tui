//! Codes, and the parts of pairing that are not specific to chess.

use std::time::Duration;

use iroh::SecretKey;
use tui_tui::games::chess::GAME as CHESS;
use tui_tui::session::{
    self, Code, CodeError, Game, Incoming, Listener, Progress, Target, rendezvous,
};

#[test]
fn generated_codes_round_trip() {
    for _ in 0..500 {
        let code = Code::generate();
        let text = code.to_string();
        assert_eq!(text.split('-').count(), 4, "{text}");
        assert_eq!(text.parse::<Code>(), Ok(code));
    }
}

#[test]
fn codes_forgive_retyping() {
    let code: Code = "42-tiger-marble-ocean".parse().unwrap();
    for typed in [
        "  42-Tiger-MARBLE-ocean \n",
        "42 tiger marble ocean",
        "42_tiger.marble - ocean",
        // Four letters are enough to tell every word apart.
        "42-tige-marb-ocea",
    ] {
        assert_eq!(typed.parse::<Code>(), Ok(code), "{typed:?}");
    }
}

#[test]
fn codes_explain_what_is_wrong() {
    assert_eq!("".parse::<Code>(), Err(CodeError::Shape));
    assert_eq!("42-tiger-marble".parse::<Code>(), Err(CodeError::Shape));
    assert_eq!(
        "142-tiger-marble-ocean".parse::<Code>(),
        Err(CodeError::Number("142".into()))
    );
    assert_eq!(
        "42-tiger-marble-oceanic".parse::<Code>(),
        Err(CodeError::Word("oceanic".into()))
    );
    // Too short to pick out one word.
    assert_eq!(
        "42-tig-marble-ocean".parse::<Code>(),
        Err(CodeError::Word("tig".into()))
    );
}

/// A joiner that cannot play the host's game backs out, and says why.
#[tokio::test(flavor = "multi_thread")]
async fn joiner_declines_a_game_it_does_not_have() {
    let code = Code::generate();
    let go = Game {
        name: "go",
        version: 1,
    };
    let host = session::bind(SecretKey::generate()).await.unwrap();
    host.online().await;
    let (tx, mut heard) = tokio::sync::mpsc::unbounded_channel();
    let listener = Listener::start(host.clone(), &[go], tx);
    listener.host(code, go, "alice", false);

    let guest = session::bind(SecretKey::generate()).await.unwrap();
    let at = Target::Addr(host.addr());
    let joined = session::join(&guest, code, &[CHESS], at, "bob", |_| {}).await;
    let why = format!("{:#}", joined.expect_err("joiner should back out"));
    assert!(why.contains("go 1"), "{why}");

    let seen = tokio::time::timeout(Duration::from_secs(30), heard.recv()).await;
    assert!(matches!(
        seen.unwrap(),
        Some(Incoming::Progress(Progress::Declined))
    ));
}

/// Publishes to and resolves from the real Mainline DHT, so it needs the
/// internet and can take a minute. Run it with `cargo test -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the internet"]
async fn code_round_trips_through_the_dht() {
    let code = Code::generate();
    let host = session::bind(SecretKey::generate()).await.unwrap();
    host.online().await;

    rendezvous::publish(code, &host.addr()).await.unwrap();
    let found = rendezvous::resolve(code).await.unwrap();
    assert_eq!(found.id, host.id());
    host.close().await;
}
