//! WebSocket `/ws`: batched (≤ 60 Hz) packet, device, metrics and alert streams.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;

use crate::dto::to_dto;
use crate::engine::Engine;

#[derive(Deserialize)]
struct ClientMsg {
    op: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    decoded: Option<bool>,
    /// "tail" (default) starts from the most recent packets; a number starts after that seq.
    #[serde(default)]
    from: Option<serde_json::Value>,
}

const MAX_BATCH: usize = 400;

pub async fn handle(socket: WebSocket, engine: Arc<Engine>) {
    let (mut sink, mut stream) = socket.split();
    let mut packets = false;
    let mut devices = false;
    let mut metrics = false;
    let mut alerts = false;
    let mut filter: Option<keelwire_core::filter::Expr> = None;
    let mut with_fields = false;
    let mut last_seq: u64 = 0;
    let mut dev_version: u64 = 0;
    let mut last_alert: u64 = 0;
    let mut last_metrics = Instant::now() - Duration::from_secs(5);
    let mut last_status = Instant::now() - Duration::from_secs(5);
    let mut iv = tokio::time::interval(Duration::from_millis(16));
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            m = stream.next() => {
                let Some(Ok(m)) = m else { return };
                let Message::Text(t) = m else { continue };
                let Ok(c) = serde_json::from_str::<ClientMsg>(&t) else {
                    let _ = sink.send(Message::Text(json!({"topic":"error","message":"bad request"}).to_string())).await;
                    continue;
                };
                let on = c.op == "subscribe";
                match c.topic.as_str() {
                    "packets" => {
                        packets = on;
                        if on {
                            match c.filter.as_deref().map(keelwire_core::filter::parse) {
                                Some(Ok(f)) => filter = f,
                                Some(Err(e)) => {
                                    let _ = sink.send(Message::Text(json!({"topic":"filter_error","message":e}).to_string())).await;
                                    filter = None;
                                }
                                None => filter = None,
                            }
                            with_fields = c.decoded.unwrap_or(false);
                            last_seq = match c.from.as_ref().and_then(|v| v.as_u64()) {
                                Some(s) => s,
                                None => engine.status().last_seq.saturating_sub(200),
                            };
                        }
                    }
                    "devices" => { devices = on; dev_version = 0; }
                    "metrics" => metrics = on,
                    "alerts" => { alerts = on; if on { last_alert = 0; } }
                    _ => {}
                }
            }
            _ = iv.tick() => {
                if packets {
                    let recs = engine.packets_after(last_seq, 5000);
                    if let Some(l) = recs.last() { last_seq = l.seq; }
                    let mut dropped = 0usize;
                    let start = if filter.is_none() && recs.len() > MAX_BATCH { dropped = recs.len() - MAX_BATCH; recs.len() - MAX_BATCH } else { 0 };
                    let mut items = Vec::new();
                    for r in &recs[start..] {
                        if let Some(d) = to_dto(&engine.db, r, filter.as_ref(), with_fields) {
                            items.push(d);
                        }
                    }
                    if items.len() > MAX_BATCH {
                        dropped += items.len() - MAX_BATCH;
                        items.drain(..items.len() - MAX_BATCH);
                    }
                    if !items.is_empty() || dropped > 0 {
                        let msg = json!({"topic":"packets","items":items,"dropped":dropped}).to_string();
                        if sink.send(Message::Text(msg)).await.is_err() { return; }
                    }
                }
                if devices {
                    let v = engine.device_version.load(Ordering::Relaxed);
                    if v != dev_version {
                        dev_version = v;
                        let msg = json!({"topic":"devices","items":engine.devices()}).to_string();
                        if sink.send(Message::Text(msg)).await.is_err() { return; }
                    }
                }
                if metrics && last_metrics.elapsed() >= Duration::from_secs(1) {
                    last_metrics = Instant::now();
                    let msg = json!({"topic":"metrics","data":engine.metrics(),"banks":engine.banks()}).to_string();
                    if sink.send(Message::Text(msg)).await.is_err() { return; }
                }
                if last_status.elapsed() >= Duration::from_secs(1) {
                    last_status = Instant::now();
                    let msg = json!({"topic":"status","data":engine.status()}).to_string();
                    if sink.send(Message::Text(msg)).await.is_err() { return; }
                }
                if alerts {
                    let a = engine.alerts_after(last_alert);
                    if let Some(l) = a.last() {
                        last_alert = l.seq;
                        let msg = json!({"topic":"alerts","items":a}).to_string();
                        if sink.send(Message::Text(msg)).await.is_err() { return; }
                    }
                }
            }
        }
    }
}
