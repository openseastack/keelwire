//! Signal K tap: connects to the `signalk-keelwire-tap` plugin's WebSocket,
//! which forwards raw NMEA 2000 messages as JSON
//! `{"prio":2,"pgn":127250,"src":5,"dst":255,"data":"00ff7f…"}` and accepts
//! the same JSON to transmit.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use keelwire_core::frame::{hex, parse_hex};
use keelwire_core::N2kMessage;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::{clock, BusEvent, CanInterface, InterfaceCaps, InterfaceError, InterfaceStatus, Outbound, Uri};

pub struct SignalK {
    url: String,
    allow_tx: bool,
}

impl SignalK {
    pub fn new(u: &Uri) -> Result<Self, InterfaceError> {
        if !(u.path.starts_with("ws://") || u.path.starts_with("wss://")) {
            return Err(InterfaceError::BadUri("signalk:ws://host:3000/plugins/signalk-keelwire-tap/stream".into()));
        }
        let mut url = u.path.clone();
        if !url.contains("/plugins/") {
            url = format!("{}/plugins/signalk-keelwire-tap/stream", url.trim_end_matches('/'));
        }
        Ok(SignalK { url, allow_tx: u.flag("tx") })
    }
}

pub fn parse_json(v: &serde_json::Value) -> Option<N2kMessage> {
    let data = parse_hex(v.get("data")?.as_str()?)?;
    Some(N2kMessage {
        timestamp_us: 0,
        priority: v.get("prio").and_then(|x| x.as_u64()).unwrap_or(6) as u8,
        pgn: v.get("pgn")?.as_u64()? as u32,
        source: v.get("src").and_then(|x| x.as_u64()).unwrap_or(0) as u8,
        destination: v.get("dst").and_then(|x| x.as_u64()).unwrap_or(255) as u8,
        data,
    })
}

impl CanInterface for SignalK {
    fn name(&self) -> String {
        format!("Signal K {}", self.url)
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps { raw_frames: false, can_transmit: self.allow_tx, tx_messages: true, ..Default::default() }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(500);
            loop {
                let _ = events.send(BusEvent::Status(InterfaceStatus::Connecting)).await;
                let reason = match tokio_tungstenite::connect_async(self.url.as_str()).await {
                    Ok((ws, _)) => {
                        backoff = Duration::from_millis(500);
                        let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: self.url.clone() })).await;
                        let (mut sink, mut stream) = ws.split();
                        loop {
                            tokio::select! {
                                m = stream.next() => match m {
                                    Some(Ok(WsMessage::Text(t))) => {
                                        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
                                        let items = if let Some(a) = v.as_array() { a.clone() } else { vec![v] };
                                        for it in items {
                                            if let Some(mut msg) = parse_json(&it) {
                                                msg.timestamp_us = clock::now_us();
                                                if events.send(BusEvent::Message(msg)).await.is_err() { return; }
                                            }
                                        }
                                    }
                                    Some(Ok(_)) => {}
                                    Some(Err(e)) => break e.to_string(),
                                    None => break "closed".to_string(),
                                },
                                o = outbound.recv() => match o {
                                    Some(Outbound::Message(m)) if self.allow_tx => {
                                        let j = serde_json::json!({"prio": m.priority, "pgn": m.pgn, "dst": m.destination, "data": hex(&m.data)});
                                        if let Err(e) = sink.send(WsMessage::Text(j.to_string())).await { break e.to_string(); }
                                    }
                                    Some(_) => {}
                                    None => return,
                                }
                            }
                        }
                    }
                    Err(e) => e.to_string(),
                };
                let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason })).await;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        })
    }
}
