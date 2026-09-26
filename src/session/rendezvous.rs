//! Turning a code into the host's address, over the Mainline DHT.
//!
//! Both sides stretch the code into the same ed25519 keypair. The host signs a
//! pkarr record with it saying where it can be reached, and the joiner looks
//! that record up by the public half. Nobody without the code can write the
//! record, and nobody can find it without guessing the code first.
//!
//! The stretch is deliberately slow (Argon2id, 64 MiB), so that walking the
//! code space and asking the DHT about every key costs far more than a code
//! lives. That is what keeps the codes short; the handshake still has the
//! final word on who is on the other end.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use argon2::{Algorithm, Argon2, Params, Version};
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use pkarr::dns::rdata::{RData, TXT};
use pkarr::{Client, Keypair, ResolvePolicy, SignedPacket, Timestamp};

use super::Code;

/// Bumped if the derivation or the record ever change shape, so that old and
/// new builds cannot find each other's records by accident.
const SALT: &[u8] = b"tui-tui rendezvous v1";

/// The record's name under the code's key.
const RECORD: &str = "_session";

/// How long a code stays good for after it is published.
pub const CODE_LIFETIME: Duration = Duration::from_hours(1);

/// A joiner may have the code before the host's record has spread, so it
/// keeps asking for this long before giving up.
const LOOKUP_PATIENCE: Duration = Duration::from_mins(1);
const LOOKUP_RETRY: Duration = Duration::from_secs(3);

/// Stretch the code into the keypair both sides agree on.
async fn keypair(code: Code) -> Result<Keypair> {
    tokio::task::spawn_blocking(move || {
        let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("valid argon2 params");
        let mut seed = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(code.to_string().as_bytes(), SALT, &mut seed)
            .map_err(|e| anyhow!("stretching the code: {e}"))?;
        Ok(Keypair::from_secret_key(&seed))
    })
    .await?
}

fn client() -> Result<Client> {
    Client::builder()
        .no_relays()
        .build()
        .context("starting the DHT client")
}

/// Announce that `addr` is the host for `code`.
///
/// # Errors
///
/// If the record cannot be built or published to the DHT.
pub async fn publish(code: Code, addr: &EndpointAddr) -> Result<()> {
    let keypair = keypair(code).await?;
    let id = format!("id={}", addr.id);
    let relay = addr.relay_urls().next().map(|url| format!("relay={url}"));

    let mut txt = TXT::new().with_string(&id)?;
    if let Some(relay) = &relay {
        txt.add_string(relay)?;
    }
    let packet = SignedPacket::builder()
        .txt(
            RECORD.try_into()?,
            txt,
            u32::try_from(CODE_LIFETIME.as_secs())?,
        )
        .sign(&keypair)?;
    client()?
        .publish(&packet)
        .await
        .context("publishing the code to the DHT")?;
    Ok(())
}

/// Find the host behind `code`, waiting a while for its record to appear.
///
/// # Errors
///
/// If the DHT cannot be reached, no record turns up in time, or the one
/// that does is stale or damaged.
pub async fn resolve(code: Code) -> Result<EndpointAddr> {
    let key = keypair(code).await?.public_key();
    let client = client()?;

    let found = tokio::time::timeout(LOOKUP_PATIENCE, async {
        loop {
            if let Ok(packet) = client.resolve(&key, ResolvePolicy::NetworkOnly).await {
                return packet;
            }
            tokio::time::sleep(LOOKUP_RETRY).await;
        }
    })
    .await
    .map_err(|_| anyhow!("no game found for that code — check it with your opponent"))?;

    let age = Timestamp::now()
        .as_u64()
        .saturating_sub(found.timestamp().as_u64());
    if Duration::from_micros(age) > CODE_LIFETIME {
        bail!("that code has expired — ask your opponent for a new one");
    }
    read_record(&found)
}

fn read_record(packet: &SignedPacket) -> Result<EndpointAddr> {
    let attrs = packet
        .resource_records(RECORD)
        .find_map(|rr| match &rr.rdata {
            RData::TXT(txt) => Some(txt.attributes()),
            _ => None,
        })
        .context("the code's record is empty")?;
    let id: EndpointId = attrs
        .get("id")
        .cloned()
        .flatten()
        .context("the code's record names no host")?
        .parse()
        .context("the code's record names a host that is not an endpoint id")?;

    let mut addr = EndpointAddr::new(id);
    // The relay is a head start, not a requirement: iroh can find the rest.
    if let Some(Some(url)) = attrs.get("relay")
        && let Ok(url) = url.parse::<RelayUrl>()
    {
        addr = addr.with_relay_url(url);
    }
    Ok(addr)
}
