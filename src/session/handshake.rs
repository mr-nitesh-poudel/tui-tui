//! What the two sides say before the game starts, one line at a time.
//!
//! The dialling side's first line picks one of two openings. Pairing by code:
//!
//! ```text
//! joiner  pake <hex>        SPAKE2, keyed by the code
//! host    pake <hex>
//! joiner  confirm <hex>     proof we reached the same key
//! host    confirm <hex>     ...or `no wrong-code`
//! joiner  name <name>
//! host    name <name>
//! host    game chess 1      what the host is playing
//! joiner  ok                ...or `no unsupported-game`
//! ```
//!
//! SPAKE2 gives someone without the code exactly one guess per connection, and
//! the host only allows a few wrong ones in total. The confirmations are keyed
//! over both endpoint ids, which iroh has already authenticated, so a man in
//! the middle relaying the exchange ends up with ids that do not match.
//!
//! Inviting a friend, whose endpoint id was learned by pairing once already.
//! iroh proves who each side is, so there is no code; the host decides from
//! its contacts whether to ask its player at all:
//!
//! ```text
//! guest   invite
//! guest   name <name>
//! guest   game chess 1
//! host    name <name>       once its player says yes
//! host    ok                ...or `no declined`, `no busy`, `no unknown`...
//! ```

use std::fmt::Write as _;
use std::time::Duration;

use super::lines::Lines;
use super::name::clean_name;
use super::{Code, Game};
use anyhow::{Context, Result, anyhow, bail};
use iroh::EndpointId;
use iroh::endpoint::{RecvStream, SendStream};
use spake2::{Ed25519Group, Identity, Password, Spake2};

/// A peer's side of the conversation, one bounded line at a time.
pub type Reader = Lines<RecvStream>;

const IDENTITY: &[u8] = b"tui-tui/session/1";

/// The peer used a different code. The host counts these; anything else going
/// wrong mid-handshake is just a dropped attempt.
#[derive(Debug)]
pub struct WrongCode;

impl std::fmt::Display for WrongCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the code did not match — check it with your opponent")
    }
}

impl std::error::Error for WrongCode {}

/// The joiner got in but cannot play the host's game.
#[derive(Debug)]
pub struct Declined;

impl std::fmt::Display for Declined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("your opponent's build cannot play this game")
    }
}

impl std::error::Error for Declined {}

/// The peer backed out with `no <reason>`, for a reason with no type of its
/// own. The reason is one word, meant for turning into a message.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "peer refused: {}", self.0)
    }
}

impl std::error::Error for Refused {}

#[derive(Clone, Copy)]
#[repr(u8)]
enum Role {
    Host = 0,
    Joiner = 1,
}

/// The host's half of pairing by code, from just after the joiner's opening
/// `pake` line. Returns the joiner's name.
///
/// # Errors
///
/// If the joiner has the wrong code, the stream fails, or they send
/// something other than the handshake.
pub async fn host_code(
    send: &mut SendStream,
    lines: &mut Reader,
    their_pake: &str,
    code: Code,
    game: Game,
    name: &str,
    ids: (EndpointId, EndpointId),
) -> Result<String> {
    let (spake, ours) = start(code);
    let theirs = unhex(their_pake)?;
    say(send, &format!("pake {}", hex(&ours))).await?;
    let key = finish(spake, &theirs)?;

    let claimed = hear(lines, "confirm").await?;
    if !matches(&claimed, tag(&key, Role::Joiner, ids)) {
        refuse(send, "wrong-code").await;
        return Err(WrongCode.into());
    }
    say(
        send,
        &format!("confirm {}", tag(&key, Role::Host, ids).to_hex()),
    )
    .await?;

    let their_name = hear_name(lines).await?;
    say(send, &format!("name {name}")).await?;
    say(send, &format!("game {} {}", game.name, game.version)).await?;
    hear(lines, "ok").await?;
    Ok(their_name)
}

/// The joiner's half of pairing by code. Returns whichever of `games` the host
/// turned out to be playing, and the host's name.
///
/// # Errors
///
/// If the code is wrong, the host is playing none of `games`, the stream
/// fails, or the host sends something other than the handshake.
pub async fn join_code(
    send: &mut SendStream,
    lines: &mut Reader,
    code: Code,
    games: &[Game],
    name: &str,
    ids: (EndpointId, EndpointId),
) -> Result<(Game, String)> {
    let (spake, ours) = start(code);
    say(send, &format!("pake {}", hex(&ours))).await?;
    let theirs = unhex(&hear(lines, "pake").await?)?;
    let key = finish(spake, &theirs)?;

    say(
        send,
        &format!("confirm {}", tag(&key, Role::Joiner, ids).to_hex()),
    )
    .await?;
    let claimed = hear(lines, "confirm").await?;
    if !matches(&claimed, tag(&key, Role::Host, ids)) {
        return Err(WrongCode.into());
    }

    say(send, &format!("name {name}")).await?;
    let their_name = hear_name(lines).await?;

    let offer = hear(lines, "game").await?;
    if let Some(game) = parse_game(&offer, games) {
        say(send, "ok").await?;
        Ok((game, their_name))
    } else {
        refuse(send, "unsupported-game").await;
        bail!("your opponent is playing {offer}, which this build does not have")
    }
}

/// The guest's half of an invite. Returns the host's name once it says yes.
///
/// # Errors
///
/// If the host says no, the stream fails, or it sends something other than
/// the handshake.
pub async fn invite(
    send: &mut SendStream,
    lines: &mut Reader,
    game: Game,
    name: &str,
) -> Result<String> {
    say(send, "invite").await?;
    say(send, &format!("name {name}")).await?;
    say(send, &format!("game {} {}", game.name, game.version)).await?;
    let their_name = hear_name(lines).await?;
    hear(lines, "ok").await?;
    Ok(their_name)
}

/// The host's first look at an invite, just after its opening line: who says
/// they are asking, and to play what.
///
/// # Errors
///
/// If the stream fails, or the guest sends something other than a name and
/// a game.
pub async fn read_invite(lines: &mut Reader, games: &[Game]) -> Result<(String, Option<Game>)> {
    let name = hear_name(lines).await?;
    let offer = hear(lines, "game").await?;
    Ok((name, parse_game(&offer, games)))
}

/// The host saying yes to an invite.
///
/// # Errors
///
/// If the stream fails.
pub async fn accept_invite(send: &mut SendStream, name: &str) -> Result<()> {
    say(send, &format!("name {name}")).await?;
    say(send, "ok").await
}

fn parse_game(offer: &str, games: &[Game]) -> Option<Game> {
    let (name, version) = offer.split_once(' ')?;
    let version: u32 = version.parse().ok()?;
    games
        .iter()
        .find(|g| g.name == name && g.version == version)
        .copied()
}

fn start(code: Code) -> (Spake2<Ed25519Group>, Vec<u8>) {
    Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(code.to_string()),
        &Identity::new(IDENTITY),
    )
}

fn finish(spake: Spake2<Ed25519Group>, theirs: &[u8]) -> Result<[u8; 32]> {
    let key = spake
        .finish(theirs)
        .map_err(|e| anyhow!("peer sent a bad key exchange: {e:?}"))?;
    key.try_into()
        .map_err(|_| anyhow!("key exchange gave a key of the wrong size"))
}

fn tag(key: &[u8; 32], role: Role, (host, joiner): (EndpointId, EndpointId)) -> blake3::Hash {
    let mut h = blake3::Hasher::new_keyed(key);
    h.update(b"confirm");
    h.update(&[role as u8]);
    h.update(host.as_bytes());
    h.update(joiner.as_bytes());
    h.finalize()
}

/// `blake3::Hash` compares in constant time, so parse and compare as hashes.
fn matches(claimed: &str, expected: blake3::Hash) -> bool {
    blake3::Hash::from_hex(claimed).is_ok_and(|h| h == expected)
}

async fn say(send: &mut SendStream, line: &str) -> Result<()> {
    send.write_all(format!("{line}\n").as_bytes())
        .await
        .context("writing to peer")?;
    Ok(())
}

/// Back out with `no <why>`, and let that land before the caller drops the
/// connection under it: closing a connection throws away unsent data.
pub async fn refuse(send: &mut SendStream, why: &str) {
    if say(send, &format!("no {why}")).await.is_ok() && send.finish().is_ok() {
        let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
    }
}

/// The next line, which has to be `verb`, returning whatever follows it. The
/// peer backing out with `no <why>` comes back as the error.
/// The peer's name, cleaned on the way in. Empty if nothing readable was
/// sent; whoever shows it falls back to the peer's endpoint id.
async fn hear_name(lines: &mut Reader) -> Result<String> {
    let name = hear(lines, "name").await?;
    Ok(clean_name(&name).unwrap_or_default())
}

/// The next line, which should start with `verb`, less that word.
///
/// # Errors
///
/// If the stream fails or ends, the peer says `no`, or it sends some other
/// verb than `verb`.
pub async fn hear(lines: &mut Reader, verb: &str) -> Result<String> {
    let line = lines
        .next_line()
        .await
        .context("reading from peer")?
        .context("peer hung up during the handshake")?;
    let (word, rest) = line.split_once(' ').unwrap_or((&line, ""));
    match word {
        w if w == verb => Ok(rest.to_string()),
        "no" if rest == "wrong-code" => Err(WrongCode.into()),
        "no" if rest == "unsupported-game" => Err(Declined.into()),
        "no" => Err(Refused(rest.to_string()).into()),
        _ => bail!("expected {verb:?} from peer, got {word:?}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(s: &str) -> Result<Vec<u8>> {
    // Checked up front so the byte slicing below cannot split a character.
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        bail!("bad hex from peer");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("bad hex from peer"))
        .collect()
}
