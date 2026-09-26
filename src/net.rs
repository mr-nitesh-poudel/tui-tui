//! Carrying a game's messages over a [`session`](crate::session) link.
//!
//! Pairing, codes and invites all happen in the session layer; by the time
//! this module sees the connection, both sides know who the other is and have
//! agreed on a game. What is left is newline-delimited text, one message a
//! line. What the lines say is up to each game; the only word reserved here
//! is `bye`, for leaving.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::EndpointId;
use iroh::endpoint::SendStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::session::{Link, Progress, Reader};

/// Something the peer did, delivered to the UI loop.
#[derive(Debug)]
pub enum NetEvent {
    /// Pairing by code moved along.
    Progress(Progress),
    /// One of the game's own messages, without its newline.
    Line(String),
    Disconnected(String),
}

/// A game in progress with a peer. Dropping it leaves the game, and the peer
/// is told so.
pub struct Net {
    pub peer: EndpointId,
    /// Lines for the peer, without their newlines.
    pub out: UnboundedSender<String>,
}

impl Net {
    pub fn send(&self, line: String) {
        let _ = self.out.send(line);
    }
}

/// Play over `link` until one side hangs up.
#[must_use]
pub fn play(link: Link, events: UnboundedSender<NetEvent>) -> Net {
    let (out_tx, out_rx) = unbounded_channel();
    let peer = link.peer;
    tokio::spawn(async move {
        let msg = match pump(link.send, link.lines, out_rx, &events).await {
            Ok(Ended::ByUs) => return,
            Ok(Ended::ByThem) => "your opponent left".to_string(),
            Err(e) => format!("lost the connection to your opponent ({e:#})"),
        };
        let _ = events.send(NetEvent::Disconnected(msg));
    });
    Net { peer, out: out_tx }
}

enum Ended {
    ByUs,
    ByThem,
}

/// Shuttle lines both ways until one side hangs up.
async fn pump(
    mut send: SendStream,
    mut lines: Reader,
    mut out_rx: UnboundedReceiver<String>,
    events: &UnboundedSender<NetEvent>,
) -> Result<Ended> {
    loop {
        tokio::select! {
            outgoing = out_rx.recv() => {
                let Some(msg) = outgoing else {
                    // Say goodbye rather than just vanishing, and give it a
                    // moment to arrive before the connection goes.
                    let _ = send.write_all(b"bye\n").await;
                    if send.finish().is_ok() {
                        let _ = tokio::time::timeout(Duration::from_secs(1), send.stopped()).await;
                    }
                    return Ok(Ended::ByUs);
                };
                send.write_all(format!("{msg}\n").as_bytes()).await.context("writing to peer")?;
            }
            incoming = lines.next_line() => {
                let Some(line) = incoming.context("reading from peer")? else {
                    return Ok(Ended::ByThem);
                };
                let line = line.trim_end();
                if line == "bye" {
                    return Ok(Ended::ByThem);
                }
                if events.send(NetEvent::Line(line.to_string())).is_err() {
                    return Ok(Ended::ByUs);
                }
            }
        }
    }
}
