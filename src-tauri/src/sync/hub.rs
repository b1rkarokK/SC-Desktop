//! WebSocket client for one channel of the relay (`/ch/<id>`): reconnects by itself, remembers
//! the last stored message it saw (`?since=`), keeps what is sent while the connection is down
//! and sends it when the connection is back.

use std::{collections::VecDeque, time::Duration};

use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone)]
pub enum HubEvent {
    /// connected; `head` is the newest stored sequence number, `peers` how many others are here
    Connected { head: u64, peers: u64 },
    /// a message: `seq` is 0 for the ones that are only relayed, `id` echoes our own send id
    Msg { seq: u64, blob: String, keep: bool, id: Option<String> },
    Disconnected,
}

#[derive(Debug)]
enum Out {
    Send { blob: String, keep: bool, id: Option<String> },
    Close,
}

#[derive(Clone)]
pub struct HubClient {
    tx: mpsc::UnboundedSender<Out>,
}

impl HubClient {
    pub fn send(&self, blob: String, keep: bool, id: Option<String>) {
        let _ = self.tx.send(Out::Send { blob, keep, id });
    }

    pub fn close(&self) {
        let _ = self.tx.send(Out::Close);
    }
}

#[derive(Deserialize)]
struct Wire {
    t: String,
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    head: u64,
    #[serde(default)]
    peers: u64,
    #[serde(default)]
    more: bool,
    #[serde(default)]
    blob: String,
    #[serde(default)]
    keep: bool,
    #[serde(default)]
    id: Option<String>,
}

pub fn ws_url(base: &str, channel: &str, since: u64) -> String {
    let base = base.trim().trim_end_matches('/');
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };
    format!("{base}/ch/{channel}?since={since}")
}

/// Connects in the background. Events come out of the receiver until `close()`.
pub fn connect(base: String, channel: String, since: u64) -> (HubClient, mpsc::UnboundedReceiver<HubEvent>) {
    let (tx, mut out_rx) = mpsc::unbounded_channel::<Out>();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel::<HubEvent>();
    tauri::async_runtime::spawn(async move {
        let mut since = since;
        let mut pending: VecDeque<Out> = VecDeque::new();
        let mut backoff = Duration::from_millis(500);
        'outer: loop {
            let url = ws_url(&base, &channel, since);
            let attempt = tokio::time::timeout(Duration::from_secs(12), tokio_tungstenite::connect_async(url)).await;
            let Ok(Ok((ws, _))) = attempt else {
                // wait, but keep taking what the app wants to send
                let sleep = tokio::time::sleep(backoff);
                tokio::pin!(sleep);
                loop {
                    tokio::select! {
                        _ = &mut sleep => break,
                        o = out_rx.recv() => match o {
                            Some(Out::Close) | None => return,
                            Some(o) => { pending.push_back(o); while pending.len() > 200 { pending.pop_front(); } }
                        },
                    }
                }
                backoff = (backoff * 2).min(Duration::from_secs(15));
                continue;
            };
            backoff = Duration::from_millis(500);
            let (mut sink, mut stream) = ws.split();
            let mut ping = tokio::time::interval(Duration::from_secs(20));
            ping.tick().await;
            let mut backlog = 0usize;
            let mut more = false;
            loop {
                tokio::select! {
                    _ = ping.tick() => {
                        if sink.send(Message::Text(r#"{"t":"ping"}"#.into())).await.is_err() { break; }
                    }
                    o = out_rx.recv() => match o {
                        Some(Out::Close) | None => { let _ = sink.close().await; break 'outer; }
                        Some(o) => pending.push_back(o),
                    },
                    m = stream.next() => {
                        let Some(Ok(m)) = m else { break };
                        let Message::Text(text) = m else { continue };
                        let Ok(w) = serde_json::from_str::<Wire>(&text) else { continue };
                        match w.t.as_str() {
                            "hello" => {
                                more = w.more;
                                let _ = ev_tx.send(HubEvent::Connected { head: w.head, peers: w.peers });
                            }
                            "msg" => {
                                if w.keep && w.seq > 0 {
                                    since = since.max(w.seq);
                                    backlog += 1;
                                }
                                let _ = ev_tx.send(HubEvent::Msg { seq: w.seq, blob: w.blob, keep: w.keep, id: w.id });
                                // the stored history is longer than one batch: ask for the rest
                                if more && backlog >= 500 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // flush what waited while we were away (also what was queued just now)
                while let Some(Out::Send { blob, keep, id }) = pending.pop_front() {
                    let msg = serde_json::json!({ "t": "send", "blob": blob, "keep": keep, "id": id }).to_string();
                    if sink.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
            }
            let _ = ev_tx.send(HubEvent::Disconnected);
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
    (HubClient { tx }, ev_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_the_scheme_of_the_server_address() {
        assert_eq!(ws_url("https://x.workers.dev/", "abc", 7), "wss://x.workers.dev/ch/abc?since=7");
        assert_eq!(ws_url("http://127.0.0.1:8787", "abc", 0), "ws://127.0.0.1:8787/ch/abc?since=0");
        assert_eq!(ws_url("wss://x.dev", "abc", 1), "wss://x.dev/ch/abc?since=1");
    }
}

#[cfg(test)]
mod live {
    use super::*;

    async fn next_msg(rx: &mut mpsc::UnboundedReceiver<HubEvent>) -> HubEvent {
        loop {
            match tokio::time::timeout(Duration::from_secs(8), rx.recv()).await {
                Ok(Some(e @ HubEvent::Msg { .. })) => return e,
                Ok(Some(_)) => continue,
                other => panic!("no message: {other:?}"),
            }
        }
    }

    /// Needs the relay: `cd server && npm run dev`, then
    /// `SYNC_URL=http://127.0.0.1:8787 cargo test sync::hub::live -- --ignored`
    #[test]
    #[ignore]
    fn two_clients_talk_through_the_relay_and_catch_up_after_a_break() {
        let Ok(base) = std::env::var("SYNC_URL") else { return };
        tauri::async_runtime::block_on(async move {
            let channel = format!("test{}", crate::sync::crypto::new_channel_id());
            let (a, mut a_rx) = connect(base.clone(), channel.clone(), 0);
            let (b, mut b_rx) = connect(base.clone(), channel.clone(), 0);
            tokio::time::sleep(Duration::from_millis(800)).await;
            a.send("hello-b".into(), false, Some("m1".into()));
            let HubEvent::Msg { blob, .. } = next_msg(&mut b_rx).await else { unreachable!() };
            assert_eq!(blob, "hello-b");
            let HubEvent::Msg { id, .. } = next_msg(&mut a_rx).await else { unreachable!() };
            assert_eq!(id.as_deref(), Some("m1"), "the sender gets its own message back");

            // b goes away; a keeps writing; b comes back with the sequence number it had
            a.send("kept-1".into(), true, None);
            let HubEvent::Msg { seq, .. } = next_msg(&mut b_rx).await else { unreachable!() };
            assert!(seq > 0);
            b.close();
            a.send("kept-2".into(), true, None);
            tokio::time::sleep(Duration::from_millis(500)).await;
            let (c, mut c_rx) = connect(base.clone(), channel.clone(), seq);
            let HubEvent::Msg { blob, .. } = next_msg(&mut c_rx).await else { unreachable!() };
            assert_eq!(blob, "kept-2", "only what is newer than ?since=");

            // a message sent while the link is down goes out when it is back (connect to a closed port first)
            let (d, mut d_rx) = connect("http://127.0.0.1:1".into(), channel.clone(), 0);
            d.send("never".into(), false, None);
            assert!(tokio::time::timeout(Duration::from_secs(2), d_rx.recv()).await.is_err(), "no server, no events");
            d.close();
            a.close();
            c.close();
        });
    }
}
