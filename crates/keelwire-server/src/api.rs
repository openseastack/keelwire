//! REST API (`/api/v1`) and router assembly.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State, WebSocketUpgrade};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use keelwire_core::frame::{hex, parse_hex};
use keelwire_core::N2kMessage;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::capture::{sanitize, CaptureFormat};
use crate::dto::to_dto;
use crate::engine::{Engine, EngineError, TxMode};

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub token: Option<String>,
}

pub struct ApiError(StatusCode, String, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1, "message": self.2}))).into_response()
    }
}

impl From<EngineError> for ApiError {
    fn from(e: EngineError) -> Self {
        let (code, kind) = match &e {
            EngineError::NotAllowed(_) => (StatusCode::FORBIDDEN, "not_allowed"),
            EngineError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            EngineError::Bad(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            EngineError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout"),
            EngineError::Device(_) => (StatusCode::BAD_GATEWAY, "device_error"),
        };
        ApiError(code, kind.into(), e.to_string())
    }
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "bad_request".into(), msg.into())
}

type R<T> = Result<T, ApiError>;

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/interfaces", get(interfaces))
        .route("/interface", post(connect).delete(disconnect))
        .route("/tx", post(set_tx))
        .route("/settings/units", get(get_units).put(put_units))
        .route("/serial-ports", get(serial_ports))
        .route("/devices", get(devices))
        .route("/devices/refresh", post(refresh))
        .route("/devices/:key", get(device))
        .route("/devices/:key/messages", get(device_messages))
        .route("/devices/:key/standard", patch(patch_standard))
        .route("/devices/:key/pgn-rate", post(pgn_rate))
        .route("/devices/:key/data-instance", post(data_instance))
        .route("/devices/:key/fields", post(command_fields))
        .route("/devices/:key/address", post(set_address))
        .route("/devices/:key/request", post(device_request))
        .route("/devices/:key/maretron", post(maretron_raw))
        .route("/devices/:key/maretron/actions", get(maretron_actions))
        .route("/devices/:key/maretron/action", post(maretron_action))
        .route("/devices/:key/tank-capacity", post(tank_capacity))
        .route("/devices/:key/config", get(get_config).put(put_config))
        .route("/switch", post(switch))
        .route("/switch/banks", get(banks))
        .route("/bus/tx", post(bus_tx))
        .route("/packets", get(packets).delete(clear_packets))
        .route("/capture/start", post(capture_start))
        .route("/capture/stop", post(capture_stop))
        .route("/captures", get(captures))
        .route("/captures/:file", get(capture_file).delete(capture_delete))
        .route("/boatfile", get(boat_get).post(boat_post).delete(boat_delete))
        .route("/analysis/instances", get(analysis))
        .route("/alerts", get(alerts))
        .route("/metrics", get(metrics))
        .route("/metrics/history", get(metrics_history))
        .route("/stats", get(stats))
        .route("/stats/reset", post(stats_reset))
        .route("/pgn-db", get(pgn_db))
        .route("/pgn-db/:pgn", get(pgn_def))
        .route("/lookup/:name", get(lookup))
        .route("/decode", post(decode))
        .route("/schemas", get(schemas))
        .route("/stream/pcap", get(stream_pcap))
        .route("/session", get(session_get).patch(session_patch))
        .route("/session/start", post(session_start))
        .route("/session/pause", post(session_pause))
        .route("/session/resume", post(session_resume))
        .route("/session/new", post(session_new))
        .route("/session/notes", post(session_note))
        .route("/session/notes/:ts", axum::routing::delete(session_note_delete))
        .route("/session/since-last", get(session_since_last))
        .route("/session/end", post(session_end))
        .route("/history", get(history))
        .route("/history/file", get(history_file))
        .route("/native/share", post(native_share))
        .route("/native/save", post(native_save).layer(axum::extract::DefaultBodyLimit::max(64 << 20)))
        .layer(middleware::from_fn_with_state(state.clone(), changelog));
    Router::new()
        .nest("/api/v1", api)
        .route("/ws", get(ws))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .fallback(crate::assets::static_handler)
        .with_state(state)
}

async fn auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(tok) = &s.token {
        let path = req.uri().path();
        if path.starts_with("/api/") || path == "/ws" {
            let hdr = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(|v| v.trim_start_matches("Bearer ").to_string());
            let q = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")).map(|s| s.to_string()));
            if hdr.as_deref() != Some(tok) && q.as_deref() != Some(tok) {
                return (StatusCode::UNAUTHORIZED, Json(json!({"error":"unauthorized","message":"missing or wrong token"}))).into_response();
            }
        }
    }
    next.run(req).await
}

/// Records every request that changes a device in the open session's change
/// log, with the device as it was before and after.
async fn changelog(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let full = if path.starts_with("/api/v1") { path.clone() } else { format!("/api/v1{path}") };
    if !crate::session::is_change_route(&method, &full) || s.engine.state.lock().session.is_none() {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 1 << 20).await else {
        return ApiError(StatusCode::PAYLOAD_TOO_LARGE, "too_large".into(), "request too large".into()).into_response();
    };
    let body_json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let key = full.trim_start_matches("/api/v1/devices/").split('/').next().unwrap_or("").to_string();
    let before = if full.starts_with("/api/v1/devices/") { s.engine.device(&key) } else { None };
    let addr = before.as_ref().and_then(|d| d.address);
    let resp = next.run(Request::from_parts(parts, Body::from(bytes))).await;
    let status = resp.status();
    let (rparts, rbody) = resp.into_parts();
    let rbytes = axum::body::to_bytes(rbody, 4 << 20).await.unwrap_or_default();
    let rjson: Value = serde_json::from_slice(&rbytes).unwrap_or(Value::Null);
    // The key changes with the device instance: look the device up by address.
    let after = addr.and_then(|a| s.engine.key_at(a)).and_then(|k| s.engine.device(&k));
    let db = s.engine.db.clone();
    let changes = crate::session::describe_change(&crate::session::ChangeCtx { db: &db, method: &method, path: &full, body: &body_json, before: before.as_ref(), after: after.as_ref(), ok: status.is_success(), response: &rjson });
    s.engine.session_record_changes(changes);
    Response::from_parts(rparts, Body::from(rbytes))
}

async fn ws(State(s): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let engine = s.engine.clone();
    upgrade.on_upgrade(move |sock| crate::ws::handle(sock, engine))
}

async fn status(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.status())
}

async fn serial_ports() -> impl IntoResponse {
    Json(tokio::task::spawn_blocking(keelwire_hal::serial_ports).await.unwrap_or_default())
}

async fn get_units() -> Json<keelwire_core::units::Units> {
    Json(keelwire_core::units::current())
}

async fn put_units(State(s): State<AppState>, Json(u): Json<keelwire_core::units::Units>) -> R<Json<Value>> {
    s.engine.set_units(u)?;
    Ok(Json(json!({"ok": true})))
}

async fn interfaces() -> impl IntoResponse {
    Json(keelwire_hal::kinds())
}

#[derive(Deserialize)]
struct ConnectReq {
    uri: String,
}

async fn connect(State(s): State<AppState>, Json(r): Json<ConnectReq>) -> R<Json<Value>> {
    s.engine.connect(&r.uri)?;
    Ok(Json(json!({"ok": true})))
}

async fn disconnect(State(s): State<AppState>) -> Json<Value> {
    s.engine.disconnect();
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct TxReq {
    mode: Option<TxMode>,
    armed: Option<bool>,
    expert: Option<bool>,
}

async fn set_tx(State(s): State<AppState>, Json(r): Json<TxReq>) -> R<Json<Value>> {
    s.engine.set_tx(r.mode, r.armed, r.expert)?;
    Ok(Json(json!({"ok": true})))
}

async fn devices(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.devices())
}

async fn refresh(State(s): State<AppState>) -> impl IntoResponse {
    s.engine.refresh();
    Json(json!({"ok": true}))
}

async fn device(State(s): State<AppState>, Path(key): Path<String>) -> R<Json<Value>> {
    let d = s.engine.device(&key).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "not_found".into(), key.clone()))?;
    let rates: Vec<Value> = {
        let st = s.engine.state.lock();
        d.observed
            .keys()
            .map(|p| json!({"pgn": p, "name": s.engine.db.name(*p).unwrap_or("Unknown"), "rate": d.address.map(|a| st.metrics.pgn_rate(*p, a)).unwrap_or(0.0)}))
            .collect()
    };
    Ok(Json(json!({"device": d, "observed_rates": rates})))
}

/// Last decoded message per PGN and per instance ("Transmitted PGNs" window):
/// a PGN sent for several instances (e.g. six 130312 temperatures) gives one
/// entry per instance.
async fn device_messages(State(s): State<AppState>, Path(key): Path<String>) -> R<Json<Value>> {
    let d = s.engine.device(&key).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "not_found".into(), key.clone()))?;
    let mut items: Vec<Value> = Vec::new();
    for (pgn, o) in &d.observed {
        let per_key: Vec<(Option<&String>, &[u8], u64)> = if o.keys.len() > 1 && o.key_data.len() == o.keys.len() {
            // Hide an occurrence the device stopped sending (e.g. after an
            // instance change) once others have been seen for 10 s without it.
            let newest = o.key_seen.iter().copied().max().unwrap_or(0);
            o.keys
                .iter()
                .zip(&o.key_data)
                .zip(&o.key_seen)
                .filter(|((_, d), t)| !d.is_empty() && newest.saturating_sub(**t) < 10_000_000)
                .map(|((k, d), t)| (Some(k), d.as_slice(), *t))
                .collect()
        } else {
            vec![(o.keys.first(), o.last_data.as_slice(), o.last_us)]
        };
        for (k, data, t) in per_key {
            let dec = s.engine.db.decode(*pgn, data);
            items.push(json!({"pgn": pgn, "name": dec.description, "key": k, "count": o.count, "last_us": t, "dst": o.destination, "data": hex(data), "fields": dec.fields}));
        }
    }
    items.sort_by_key(|v| (v["pgn"].as_u64(), v["key"].as_str().map(str::to_string)));
    Ok(Json(json!(items)))
}

#[derive(Deserialize)]
struct StandardReq {
    device_instance: Option<u8>,
    system_instance: Option<u8>,
    installation_description1: Option<String>,
    installation_description2: Option<String>,
}

async fn patch_standard(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<StandardReq>) -> R<Json<Value>> {
    let e = &s.engine;
    let addr = e.device_address(&key)?;
    let mut key = key;
    if let Some(i) = r.device_instance {
        e.set_device_instance(&key, i).await?;
        key = e.key_at(addr).unwrap_or(key); // NAME (and so the key) changes with the instance
    }
    if let Some(i) = r.system_instance {
        if i > 15 {
            return Err(bad("system instance must be 0..=15"));
        }
        e.set_system_instance(&key, i).await?;
        key = e.key_at(addr).unwrap_or(key);
    }
    if let Some(t) = r.installation_description1 {
        e.set_installation(&key, 1, &t).await?;
    }
    if let Some(t) = r.installation_description2 {
        e.set_installation(&key, 2, &t).await?;
    }
    Ok(Json(json!({"ok": true, "key": key, "device": e.device(&key)})))
}

#[derive(Deserialize)]
struct RateReq {
    pgn: u32,
    /// milliseconds; 0 = disable; null = restore default
    interval_ms: Option<u32>,
}

async fn pgn_rate(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<RateReq>) -> R<Json<Value>> {
    let iv = r.interval_ms.unwrap_or(keelwire_core::groupfunction::INTERVAL_RESTORE_DEFAULT);
    s.engine.set_pgn_interval(&key, r.pgn, iv).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct DataInstanceReq {
    pgn: u32,
    #[serde(default = "one")]
    field: u8,
    new: u8,
}

fn one() -> u8 {
    1
}

#[derive(Deserialize)]
struct FieldValue {
    field: u8,
    value: f64,
}

#[derive(Deserialize)]
struct FieldsReq {
    pgn: u32,
    #[serde(default)]
    select: Vec<FieldValue>,
    values: Vec<FieldValue>,
}

async fn command_fields(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<FieldsReq>) -> R<Json<Value>> {
    let sel: Vec<(u8, f64)> = r.select.iter().map(|f| (f.field, f.value)).collect();
    let vals: Vec<(u8, f64)> = r.values.iter().map(|f| (f.field, f.value)).collect();
    let confirmed = s.engine.command_fields(&key, r.pgn, &sel, &vals).await?;
    Ok(Json(json!({"ok": true, "confirmed": confirmed})))
}

async fn lookup(State(s): State<AppState>, Path(name): Path<String>) -> R<Json<Value>> {
    let t = s.engine.db.lookups.get(&name).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "not_found".into(), name.clone()))?;
    let mut v: Vec<(i64, String)> = t.iter().map(|(k, v)| (*k, v.clone())).collect();
    v.sort();
    Ok(Json(json!(v)))
}

async fn data_instance(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<DataInstanceReq>) -> R<Json<Value>> {
    s.engine.set_data_instance(&key, r.pgn, r.field, r.new).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct AddrReq {
    address: u8,
}

async fn set_address(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<AddrReq>) -> R<Json<Value>> {
    s.engine.set_address(&key, r.address)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct PgnReq {
    pgn: u32,
}

async fn device_request(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<PgnReq>) -> R<Json<Value>> {
    let m = s.engine.iso_request(&key, r.pgn).await?;
    let dec = s.engine.db.decode(m.pgn, &m.data);
    Ok(Json(json!({"pgn": m.pgn, "data": hex(&m.data), "decoded": dec})))
}

#[derive(Deserialize)]
struct MaretronReq {
    command: u8,
    #[serde(default)]
    data: String,
}

/// Expert tool: send any Maretron command (wrapped in 126208) and show the reply.
async fn maretron_raw(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<MaretronReq>) -> R<Json<Value>> {
    if !s.engine.state.lock().expert {
        return Err(ApiError(StatusCode::FORBIDDEN, "not_allowed".into(), "raw Maretron commands require expert mode".into()));
    }
    let data = parse_hex(&r.data).ok_or_else(|| bad("data must be hex"))?;
    let reply = s.engine.maretron_command(&key, r.command, data, true).await?;
    Ok(Json(reply_json(&reply)))
}

fn reply_json(r: &crate::engine::MaretronReply) -> Value {
    json!({
        "acknowledged": r.acknowledged,
        "response": r.response.as_ref().map(|c| json!({"command": c.opcode, "product_code": c.product_code, "software_code": c.software_code, "data": hex(&c.payload)})),
        "decoded": r.decoded,
    })
}

async fn maretron_actions(State(s): State<AppState>, Path(key): Path<String>) -> R<Json<Value>> {
    let d = s.engine.device(&key).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "not_found".into(), key.clone()))?;
    if d.name.map(|n| n.manufacturer_code) != Some(137) {
        return Ok(Json(json!([])));
    }
    let pc = d.product.and_then(|p| p.product_code).unwrap_or(0);
    Ok(Json(json!(crate::maretron_actions::for_product(pc))))
}

#[derive(Deserialize)]
struct ActionReq {
    id: String,
    value: Option<f64>,
    value2: Option<f64>,
}

async fn maretron_action(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<ActionReq>) -> R<Json<Value>> {
    let reply = s.engine.maretron_action(&key, &r.id, r.value, r.value2).await?;
    Ok(Json(reply_json(&reply)))
}

#[derive(Deserialize)]
struct CapacityReq {
    litres: f64,
}

async fn tank_capacity(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<CapacityReq>) -> R<Json<Value>> {
    let confirmed = s.engine.set_tank_capacity(&key, r.litres).await?;
    Ok(Json(json!({"ok": true, "confirmed": confirmed})))
}

/// Schema-backed proprietary configuration read.
async fn get_config(State(s): State<AppState>, Path(key): Path<String>) -> R<Json<Value>> {
    let d = s.engine.device(&key).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "not_found".into(), key.clone()))?;
    let n = d.name.ok_or_else(|| bad("device has no NAME"))?;
    let pc = d.product.as_ref().and_then(|p| p.product_code);
    let schemas: Vec<keelwire_core::proprietary::schema::Schema> = {
        let st = s.engine.state.lock();
        st.schemas.schemas.iter().filter(|x| x.manufacturer == n.manufacturer_code && (x.product_code.is_none() || x.product_code == pc) && x.read.is_some()).cloned().collect()
    };
    let standard = json!({
        "device_instance": n.device_instance(),
        "system_instance": n.system_instance,
        "installation_description1": d.config.as_ref().map(|c| c.installation_description1.clone()),
        "installation_description2": d.config.as_ref().map(|c| c.installation_description2.clone()),
        "manufacturer_information": d.config.as_ref().map(|c| c.manufacturer_information.clone()),
    });
    if schemas.is_empty() {
        return Ok(Json(json!({"standard": standard, "proprietary": null})));
    }
    let mut sections = Vec::new();
    for sc in schemas {
        let rd = sc.read.clone().unwrap_or_default();
        match s.engine.maretron_command(&key, rd.command, rd.prefix_bytes(), false).await {
            Ok(reply) => match reply.response {
                Some(resp) => match sc.decode(&resp.payload) {
                    Ok(rec) => sections.push(json!({"schema": sc.id, "verified": sc.verified, "writable": sc.write.is_some(), "values": keelwire_core::proprietary::schema::record_to_json(&rec), "raw": hex(&resp.payload)})),
                    Err(e) => sections.push(json!({"schema": sc.id, "verified": sc.verified, "error": e.to_string(), "raw": hex(&resp.payload)})),
                },
                None => sections.push(json!({"schema": sc.id, "verified": sc.verified, "error": "the device acknowledged but sent no data"})),
            },
            Err(e) => sections.push(json!({"schema": sc.id, "verified": sc.verified, "error": e.to_string()})),
        }
    }
    Ok(Json(json!({"standard": standard, "proprietary": sections})))
}

#[derive(Deserialize)]
struct PutConfigReq {
    schema: String,
    values: Value,
    #[serde(default)]
    experimental: bool,
}

async fn put_config(State(s): State<AppState>, Path(key): Path<String>, Json(r): Json<PutConfigReq>) -> R<Json<Value>> {
    let schema = s.engine.state.lock().schemas.by_id(&r.schema).cloned().ok_or_else(|| bad("unknown schema"))?;
    if !schema.verified && !(r.experimental && s.engine.state.lock().expert) {
        return Err(ApiError(StatusCode::FORBIDDEN, "unverified_schema".into(), "this layout has not been verified on hardware; enable expert mode and experimental writes to proceed".into()));
    }
    let wr = schema.write.clone().ok_or_else(|| bad("this layout is read-only"))?;
    let rec = keelwire_core::proprietary::schema::record_from_json(&r.values);
    let mut data = wr.prefix_bytes();
    data.extend(schema.encode(&rec).map_err(|e| bad(e.to_string()))?);
    let reply = s.engine.maretron_command(&key, wr.command, data, true).await?;
    Ok(Json(json!({"ok": true, "reply": reply_json(&reply)})))
}

#[derive(Deserialize)]
struct SwitchReq {
    instance: u8,
    indicator: u8,
    on: bool,
}

async fn switch(State(s): State<AppState>, Json(r): Json<SwitchReq>) -> R<Json<Value>> {
    s.engine.switch(r.instance, r.indicator, r.on)?;
    Ok(Json(json!({"ok": true})))
}

async fn banks(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.banks())
}

#[derive(Deserialize)]
struct TxFrameReq {
    can_id: Option<String>,
    pgn: Option<u32>,
    #[serde(default = "six")]
    prio: u8,
    #[serde(default = "bcast")]
    dst: u8,
    data: String,
}

fn six() -> u8 {
    6
}
fn bcast() -> u8 {
    255
}

async fn bus_tx(State(s): State<AppState>, Json(r): Json<TxFrameReq>) -> R<Json<Value>> {
    let data = parse_hex(&r.data).ok_or_else(|| bad("data must be hex"))?;
    let msg = if let Some(id) = r.can_id {
        let raw = u32::from_str_radix(id.trim_start_matches("0x"), 16).map_err(|_| bad("can_id must be hex"))?;
        let c = keelwire_core::CanId::decode(raw);
        N2kMessage::new(c.priority, c.pgn, 0, c.destination, data)
    } else {
        N2kMessage::new(r.prio, r.pgn.ok_or_else(|| bad("pgn or can_id required"))?, 0, r.dst, data)
    };
    s.engine.raw_tx(msg)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct PacketsQ {
    after_seq: Option<u64>,
    limit: Option<usize>,
    filter: Option<String>,
    fields: Option<bool>,
}

async fn packets(State(s): State<AppState>, Query(q): Query<PacketsQ>) -> R<Json<Value>> {
    let f = match q.filter.as_deref() {
        Some(t) => keelwire_core::filter::parse(t).map_err(bad)?,
        None => None,
    };
    let limit = q.limit.unwrap_or(500).min(10_000);
    let recs = match q.after_seq {
        Some(a) => s.engine.packets_after(a, 50_000),
        None => s.engine.packets_tail(50_000),
    };
    let mut items: Vec<_> = recs.iter().filter_map(|r| to_dto(&s.engine.db, r, f.as_ref(), q.fields.unwrap_or(false))).collect();
    if q.after_seq.is_none() && items.len() > limit {
        items.drain(..items.len() - limit);
    } else {
        items.truncate(limit);
    }
    Ok(Json(json!({"items": items})))
}

async fn clear_packets(State(s): State<AppState>) -> impl IntoResponse {
    s.engine.clear_packets();
    Json(json!({"ok": true}))
}

#[derive(Deserialize, Default)]
struct CaptureReq {
    #[serde(default)]
    format: CaptureFormat,
    name: Option<String>,
}

async fn capture_start(State(s): State<AppState>, body: Option<Json<CaptureReq>>) -> R<Json<Value>> {
    let r = body.map(|b| b.0).unwrap_or_default();
    let file = s.engine.capture_start(r.format, r.name)?;
    Ok(Json(json!({"ok": true, "file": file})))
}

async fn capture_stop(State(s): State<AppState>) -> impl IntoResponse {
    Json(json!({"ok": true, "capture": s.engine.capture_stop()}))
}

async fn captures(State(s): State<AppState>) -> impl IntoResponse {
    let dir = s.engine.config.data_dir.join("captures");
    let mut v: Vec<Value> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    m.is_file().then(|| json!({"file": e.file_name().to_string_lossy(), "bytes": m.len(), "modified": m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs())}))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    Json(json!({"dir": dir, "files": v}))
}

fn capture_path(s: &AppState, file: &str) -> R<std::path::PathBuf> {
    let clean = sanitize(file);
    if clean != file || clean.is_empty() {
        return Err(bad("invalid file name"));
    }
    Ok(s.engine.config.data_dir.join("captures").join(clean))
}

async fn capture_file(State(s): State<AppState>, Path(file): Path<String>) -> R<Response> {
    let p = capture_path(&s, &file)?;
    let data = tokio::fs::read(&p).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, "not_found".into(), file.clone()))?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}\""))
        .body(Body::from(data))
        .unwrap())
}

async fn capture_delete(State(s): State<AppState>, Path(file): Path<String>) -> R<Json<Value>> {
    let p = capture_path(&s, &file)?;
    tokio::fs::remove_file(&p).await.map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct BoatQ {
    vessel: Option<String>,
}

async fn boat_get(State(s): State<AppState>, Query(q): Query<BoatQ>) -> Response {
    let b = s.engine.boat_snapshot(q.vessel.clone());
    let name = sanitize(&q.vessel.unwrap_or_else(|| "vessel".into()));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}.keelwire\""))
        .body(Body::from(serde_json::to_vec_pretty(&b).unwrap_or_default()))
        .unwrap()
}

async fn boat_post(State(s): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> R<Json<Value>> {
    let b: crate::boatfile::BoatFile = serde_json::from_slice(&body).map_err(|e| bad(format!("invalid boat file: {e}")))?;
    b.validate().map_err(bad)?;
    let name = headers.get("x-file-name").and_then(|v| v.to_str().ok()).map(|s| s.to_string()).or_else(|| b.vessel.clone()).unwrap_or_else(|| "boat file".into());
    let n = b.devices.len();
    s.engine.boat_load(b, name);
    Ok(Json(json!({"ok": true, "devices": n})))
}

async fn boat_delete(State(s): State<AppState>) -> impl IntoResponse {
    s.engine.boat_close();
    Json(json!({"ok": true}))
}

async fn analysis(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.analysis())
}

#[derive(Deserialize)]
struct AfterQ {
    after: Option<u64>,
}

async fn alerts(State(s): State<AppState>, Query(q): Query<AfterQ>) -> impl IntoResponse {
    Json(s.engine.alerts_after(q.after.unwrap_or(0)))
}

#[derive(Deserialize)]
struct HistQ {
    seconds: Option<usize>,
}

async fn metrics_history(State(s): State<AppState>, Query(q): Query<HistQ>) -> impl IntoResponse {
    Json(s.engine.load_history(q.seconds.unwrap_or(600).min(3600)))
}

async fn metrics(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.metrics())
}

/// Cumulative traffic per sender and per (PGN, sender) since `since_us`
/// (engine clock, same as `now_us`).
async fn stats(State(s): State<AppState>) -> impl IntoResponse {
    let (t, now) = s.engine.traffic();
    Json(json!({ "since_us": t.since_us, "now_us": now, "sources": t.sources, "pgns": t.pgns }))
}

async fn stats_reset(State(s): State<AppState>) -> impl IntoResponse {
    s.engine.reset_traffic();
    StatusCode::NO_CONTENT
}

async fn pgn_db(State(s): State<AppState>) -> impl IntoResponse {
    let mut v: Vec<Value> = s
        .engine
        .db
        .by_pgn
        .iter()
        .flat_map(|(p, defs)| defs.iter().filter(|d| !d.fallback).map(move |d| json!({"pgn": p, "id": d.id, "name": d.description, "type": d.kind, "interval_ms": d.interval_ms})))
        .collect();
    v.sort_by_key(|x| x["pgn"].as_u64());
    Json(json!({"version": s.engine.db.version, "pgns": v}))
}

async fn pgn_def(State(s): State<AppState>, Path(pgn): Path<u32>) -> R<Json<Value>> {
    let defs = s.engine.db.get(pgn).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "not_found".into(), pgn.to_string()))?;
    let v: Vec<Value> = defs
        .iter()
        .map(|d| {
            json!({
                "id": d.id, "name": d.description, "type": d.kind, "priority": d.priority, "length": d.length, "interval_ms": d.interval_ms, "fallback": d.fallback,
                "fields": d.fields.iter().map(|f| json!({"order": f.order, "id": f.id, "name": f.name, "type": f.field_type, "bits": f.bit_length, "resolution": f.resolution, "unit": f.unit, "signed": f.signed, "lookup": f.lookup, "match": f.match_value, "description": f.description, "primary_key": f.primary_key, "offset": f.offset})).collect::<Vec<_>>()
            })
        })
        .collect();
    Ok(Json(json!(v)))
}

#[derive(Deserialize)]
struct DecodeReq {
    pgn: u32,
    data: String,
}

async fn decode(State(s): State<AppState>, Json(r): Json<DecodeReq>) -> R<Json<Value>> {
    let data = parse_hex(&r.data).ok_or_else(|| bad("data must be hex"))?;
    Ok(Json(json!(s.engine.db.decode(r.pgn, &data))))
}

async fn schemas(State(s): State<AppState>) -> impl IntoResponse {
    let st = s.engine.state.lock();
    Json(json!(st.schemas.schemas.iter().map(|x| json!({"id": x.id, "description": x.description, "model": x.model, "pgn": x.pgn, "opcodes": x.opcodes, "verified": x.verified, "source": x.source})).collect::<Vec<_>>()))
}

/// Live classic-pcap stream of every CAN frame (used by the Wireshark extcap
/// `--source http://host:port` mode and `curl … | wireshark -k -i -`).
async fn stream_pcap(State(s): State<AppState>) -> Response {
    let mut rx = s.engine.frames.subscribe();
    let (tx, body_rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(256);
    tokio::spawn(async move {
        let mut hdr = Vec::new();
        let _ = keelwire_core::pcap::PcapWriter::new(&mut hdr);
        if tx.send(Ok(hdr)).await.is_err() {
            return;
        }
        loop {
            match rx.recv().await {
                Ok(f) => {
                    let mut buf = Vec::with_capacity(32);
                    {
                        let mut w = RawPcapRecord(&mut buf);
                        w.write(keelwire_hal::clock::to_unix_us(f.timestamp_us), &f);
                    }
                    if tx.send(Ok(buf)).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    });
    let stream = futures_util::stream::unfold(body_rx, |mut rx| async move { rx.recv().await.map(|x| (x, rx)) });
    Response::builder().header(header::CONTENT_TYPE, "application/vnd.tcpdump.pcap").body(Body::from_stream(stream)).unwrap()
}

struct RawPcapRecord<'a>(&'a mut Vec<u8>);

impl RawPcapRecord<'_> {
    fn write(&mut self, unix_us: u64, f: &keelwire_core::RawCanFrame) {
        let b = keelwire_core::pcap::socketcan_bytes(f);
        self.0.extend_from_slice(&((unix_us / 1_000_000) as u32).to_le_bytes());
        self.0.extend_from_slice(&((unix_us % 1_000_000) as u32).to_le_bytes());
        self.0.extend_from_slice(&16u32.to_le_bytes());
        self.0.extend_from_slice(&16u32.to_le_bytes());
        self.0.extend_from_slice(&b);
    }
}

// ------------------------------------------------------------------ sessions

async fn session_get(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.session_status())
}

async fn session_start(State(s): State<AppState>) -> R<Json<Value>> {
    s.engine.session_start()?;
    Ok(Json(s.engine.session_status()))
}

async fn session_pause(State(s): State<AppState>) -> R<Json<Value>> {
    s.engine.session_pause()?;
    Ok(Json(s.engine.session_status()))
}

async fn session_resume(State(s): State<AppState>) -> R<Json<Value>> {
    s.engine.session_resume()?;
    Ok(Json(s.engine.session_status()))
}

async fn session_new(State(s): State<AppState>) -> R<Json<Value>> {
    s.engine.session_new().await?;
    Ok(Json(s.engine.session_status()))
}

async fn session_patch(State(s): State<AppState>, Json(p): Json<crate::engine::SessionPatch>) -> R<Json<Value>> {
    s.engine.session_update(p)?;
    Ok(Json(s.engine.session_status()))
}

#[derive(Deserialize)]
struct NoteReq {
    text: String,
}

async fn session_note(State(s): State<AppState>, Json(r): Json<NoteReq>) -> R<Json<Value>> {
    s.engine.session_note(&r.text)?;
    Ok(Json(s.engine.session_status()))
}

async fn session_note_delete(State(s): State<AppState>, Path(ts): Path<u64>) -> R<Json<Value>> {
    s.engine.session_delete_note(ts)?;
    Ok(Json(s.engine.session_status()))
}

async fn session_since_last(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.since_last())
}

async fn session_end(State(s): State<AppState>, body: Option<Json<crate::engine::EndOptions>>) -> R<Json<Value>> {
    let opts = body.map(|b| b.0).unwrap_or_default();
    let r = s.engine.session_end(opts).await?;
    Ok(Json(json!(r)))
}

async fn history(State(s): State<AppState>) -> impl IntoResponse {
    let root = s.engine.sessions_dir();
    let list = tokio::task::spawn_blocking(move || crate::session::list_history(&root)).await.unwrap_or_default();
    Json(json!({"dir": s.engine.sessions_dir(), "vessels": list}))
}

#[derive(Deserialize)]
struct FileQ {
    path: String,
    #[serde(default)]
    inline: bool,
}

/// A file from the history folder: `path` = "<boat folder>/<session>/<file>".
async fn history_file(State(s): State<AppState>, Query(q): Query<FileQ>) -> R<Response> {
    let rel = std::path::Path::new(&q.path);
    if q.path.is_empty() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(bad("invalid path"));
    }
    let p = s.engine.sessions_dir().join(rel);
    let data = tokio::fs::read(&p).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, "not_found".into(), q.path.clone()))?;
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mime = mime_guess::from_path(&p).first_or_octet_stream().to_string();
    let disp = if q.inline { "inline" } else { "attachment" };
    let ascii: String = name.chars().map(|c| if c.is_ascii() && c != '"' { c } else { '_' }).collect();
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, format!("{disp}; filename=\"{ascii}\""))
        .body(Body::from(data))
        .unwrap())
}

#[derive(Deserialize)]
struct ShareReq {
    /// History paths: "<boat folder>/<session>/<file>".
    #[serde(default)]
    paths: Vec<String>,
    /// Capture files (Traffic → Record).
    #[serde(default)]
    captures: Vec<String>,
}

fn host_platform(s: &AppState) -> R<Arc<dyn crate::engine::Platform>> {
    s.engine.platform.get().cloned().ok_or_else(|| ApiError(StatusCode::NOT_IMPLEMENTED, "not_supported".into(), "sharing is not available here; download the file instead".into()))
}

fn safe_rel(p: &str) -> R<&std::path::Path> {
    let rel = std::path::Path::new(p);
    if p.is_empty() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(bad("invalid path"));
    }
    Ok(rel)
}

/// Present the host app's share sheet (iPhone / iPad) for saved files.
async fn native_share(State(s): State<AppState>, Json(r): Json<ShareReq>) -> R<Json<Value>> {
    let platform = host_platform(&s)?;
    let mut paths = Vec::new();
    for p in &r.paths {
        paths.push(s.engine.sessions_dir().join(safe_rel(p)?));
    }
    for c in &r.captures {
        paths.push(capture_path(&s, c)?);
    }
    if let Some(missing) = paths.iter().find(|p| !p.exists()) {
        return Err(ApiError(StatusCode::NOT_FOUND, "not_found".into(), missing.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    }
    platform.share_files(paths).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct SaveQ {
    name: String,
}

/// Save a file made in the interface (CSV export, device settings…) next to
/// the session history ("Exports") and offer it in the share sheet. Used where
/// a browser would download, since app web views on iOS cannot.
async fn native_save(State(s): State<AppState>, Query(q): Query<SaveQ>, body: axum::body::Bytes) -> R<Json<Value>> {
    let platform = host_platform(&s)?;
    let name = sanitize(&q.name);
    if name.is_empty() {
        return Err(bad("invalid file name"));
    }
    let root = s.engine.sessions_dir();
    let dir = root.parent().map(|p| p.join("Exports")).unwrap_or_else(|| root.join("Exports"));
    tokio::fs::create_dir_all(&dir).await.map_err(|e| bad(e.to_string()))?;
    let path = dir.join(&name);
    tokio::fs::write(&path, &body).await.map_err(|e| bad(e.to_string()))?;
    platform.share_files(vec![path]).map_err(bad)?;
    Ok(Json(json!({"ok": true, "file": name})))
}
