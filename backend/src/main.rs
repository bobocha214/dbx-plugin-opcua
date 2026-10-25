//! DBX OPC UA plugin — native sidecar (stdio JSON-RPC 2.0).
//!
//! Protocol (per DBX plugin host contract):
//!   * Reads one JSON-RPC request per line from stdin.
//!   * Writes one JSON-RPC response per line to stdout.
//!   * All human/log output goes to stderr (never stdout).
//!
//! Methods implemented:
//!   * plugin/initialize        -> self-identify (id/version must match manifest.json)
//!   * connection/test          -> probe an OPC UA endpoint, connect then close
//!   * connection/connect       -> open a session, keep it alive keyed by connection.id
//!   * connection/disconnect    -> drop the session for connection.id
//!   * opcua/root               -> the three standard address-space roots (Objects/Types/Views)
//!   * opcua/browse             -> children of a node
//!   * opcua/read               -> Value attribute of a node
//!   * opcua/readMany           -> Value attribute of many nodes in one round trip
//!   * opcua/write              -> write a scalar Value to a node
//!
//! Uses async-opcua (pure Rust, no OpenSSL) so it compiles natively on Windows.

use std::collections::HashMap;
use std::io::Write as IoWrite;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use opcua_client::{ClientBuilder, IdentityToken, Password, Session};
use opcua_types::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

// Must match manifest.json `id` / `version`, otherwise the host rejects the sidecar with
// "Sidecar identity or protocol does not match manifest".
const PLUGIN_ID: &str = "io.github.bobocha214.opcua";
const PLUGIN_VERSION: &str = "0.1.0";
const PROTOCOL_VERSION: u32 = 1;

/// A live OPC UA session plus the task that drives its event loop.
struct ConnSession {
    session: Arc<Session>,
    _loop_handle: tokio::task::JoinHandle<StatusCode>,
}

type Sessions = Arc<Mutex<HashMap<String, ConnSession>>>;

#[derive(serde::Deserialize)]
struct ConnectionFields {
    id: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    /// Sensitive fields declared with `"binding": "secret"` may arrive under `secret`.
    #[serde(default)]
    secret: Option<String>,
}

fn log_err(msg: String) {
    eprintln!("[dbx-opcua-sidecar] error: {}", msg);
}

/// Build an OPC UA endpoint URL + identity token from connection fields.
fn endpoint_url(conn: &ConnectionFields) -> String {
    format!("opc.tcp://{}:{}", conn.host, conn.port)
}

fn identity_for(conn: &ConnectionFields) -> IdentityToken {
    // The manifest binds the password field to `secret`, so accept either `password` or `secret`.
    let pass = conn
        .password
        .clone()
        .or_else(|| conn.secret.clone())
        .unwrap_or_default();
    match conn.username.as_ref() {
        Some(u) if !u.is_empty() => IdentityToken::UserName(u.clone(), Password(pass)),
        _ => IdentityToken::Anonymous,
    }
}

/// Establish a session, returning it (and the event-loop handle) without storing it.
async fn open_session(conn: &ConnectionFields) -> Result<(Arc<Session>, tokio::task::JoinHandle<StatusCode>), String> {
    let url = endpoint_url(conn);
    let mut client = ClientBuilder::new()
        .application_name("dbx-opcua-sidecar")
        .application_uri("urn:dbx:opcua-sidecar")
        .create_sample_keypair(true)
        .trust_server_certs(true)
        .session_retry_limit(0) // fail fast on a bad endpoint
        .client()
        .map_err(|e| format!("Failed to build OPC UA client: {:?}", e))?;

    let (session, event_loop) = client
        .connect_to_matching_endpoint(url.as_str(), identity_for(conn))
        .await
        .map_err(|e| format!("Cannot connect to {}: {}", url, e))?;

    // SessionEventLoop::spawn() launches the loop onto the tokio runtime and returns a handle.
    let handle = event_loop.spawn();
    Ok((session, handle))
}

/// Extract a connection id from the generic request params.
/// Accepts `id`, `connectionId` (camelCase — what the official template's UI sends),
/// or a nested `connection.id`.
fn extract_conn_id(params: &Value) -> Option<String> {
    for key in ["id", "connectionId"] {
        if let Some(id) = params.get(key).and_then(|v| v.as_str()) {
            return Some(id.to_string());
        }
    }
    params
        .get("connection")
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Clone the live session for a connection id out of the lock (so we can await network IO).
fn clone_session(conn_id: &str, sessions: &Sessions) -> Result<Arc<Session>, String> {
    let guard = sessions.lock().unwrap();
    match guard.get(conn_id) {
        Some(c) => Ok(c.session.clone()),
        None => Err(format!(
            "Not connected to connection '{}'. Call connection/connect first.",
            conn_id
        )),
    }
}

fn node_class_name(nc: NodeClass) -> &'static str {
    match nc {
        NodeClass::Object => "Object",
        NodeClass::Variable => "Variable",
        NodeClass::Method => "Method",
        NodeClass::ObjectType => "ObjectType",
        NodeClass::VariableType => "VariableType",
        NodeClass::ReferenceType => "ReferenceType",
        NodeClass::DataType => "DataType",
        NodeClass::View => "View",
        _ => "Unspecified",
    }
}

/// Map a Variant to (dataType name, JSON value) for the frontend.
fn variant_to_json(v: &Variant) -> (String, Value) {
    match v {
        Variant::Boolean(x) => ("Boolean".into(), json!(x)),
        Variant::SByte(x) => ("SByte".into(), json!(x)),
        Variant::Byte(x) => ("Byte".into(), json!(x)),
        Variant::Int16(x) => ("Int16".into(), json!(x)),
        Variant::UInt16(x) => ("UInt16".into(), json!(x)),
        Variant::Int32(x) => ("Int32".into(), json!(x)),
        Variant::UInt32(x) => ("UInt32".into(), json!(x)),
        Variant::Int64(x) => ("Int64".into(), json!(x)),
        Variant::UInt64(x) => ("UInt64".into(), json!(x)),
        Variant::Float(x) => ("Float".into(), json!(x)),
        Variant::Double(x) => ("Double".into(), json!(x)),
        Variant::String(s) => ("String".into(), json!(s.as_ref())),
        Variant::DateTime(dt) => ("DateTime".into(), json!(dt.to_string())),
        Variant::Guid(g) => ("Guid".into(), json!(format!("{:?}", g))),
        Variant::ByteString(bs) => ("ByteString".into(), json!(format!("{:?}", bs))),
        other => ("<unsupported>".into(), json!(format!("{:?}", other))),
    }
}

/// Parse a scalar value string into a Variant for writing.
fn parse_variant(data_type: &str, value: &str) -> Result<Variant, String> {
    let bad = |e: &dyn std::fmt::Display| format!("Invalid {} value '{}': {}", data_type, value, e);
    match data_type {
        "Boolean" => value.parse::<bool>().map(Variant::Boolean).map_err(|e| bad(&e)),
        "SByte" => value.parse::<i8>().map(Variant::SByte).map_err(|e| bad(&e)),
        "Byte" => value.parse::<u8>().map(Variant::Byte).map_err(|e| bad(&e)),
        "Int16" => value.parse::<i16>().map(Variant::Int16).map_err(|e| bad(&e)),
        "UInt16" => value.parse::<u16>().map(Variant::UInt16).map_err(|e| bad(&e)),
        "Int32" => value.parse::<i32>().map(Variant::Int32).map_err(|e| bad(&e)),
        "UInt32" => value.parse::<u32>().map(Variant::UInt32).map_err(|e| bad(&e)),
        "Int64" => value.parse::<i64>().map(Variant::Int64).map_err(|e| bad(&e)),
        "UInt64" => value.parse::<u64>().map(Variant::UInt64).map_err(|e| bad(&e)),
        "Float" => value.parse::<f32>().map(Variant::Float).map_err(|e| bad(&e)),
        "Double" => value.parse::<f64>().map(Variant::Double).map_err(|e| bad(&e)),
        "String" => Ok(Variant::String(UAString::from(value.to_string()))),
        other => Err(format!("Unsupported data type for write: {}", other)),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Sidecar Protocol v1 handshake. The host sends its supported versions in
/// `params.host.protocolVersions`; the reply must carry `protocolVersion`, `capabilities`
/// and `plugin { id, version }` matching manifest.json.
///
/// IMPORTANT: `capabilities` is a **sequence of strings** (the host deserializes it as
/// `Vec<String>`). Sending an object here makes the host fail with
/// "invalid type: map, expected a sequence". It mirrors the official SDK's
/// `PluginMetadata::new(..).with_capability("connections")`.
/// The `plugin` object carries only `id` and `version` — exactly what the SDK emits.
fn handle_initialize(params: &Value) -> Result<Value, String> {
    if let Some(versions) = params
        .get("host")
        .and_then(|h| h.get("protocolVersions"))
        .and_then(|v| v.as_array())
    {
        let supported = versions
            .iter()
            .any(|v| v.as_u64() == Some(PROTOCOL_VERSION as u64));
        if !supported {
            return Err(format!(
                "DBX and plugin do not share a protocol version (host offers {:?}, plugin speaks v{})",
                versions, PROTOCOL_VERSION
            ));
        }
    }

    Ok(json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": ["connections"],
        "plugin": {
            "id": PLUGIN_ID,
            "version": PLUGIN_VERSION
        }
    }))
}

async fn handle_test(params: &Value, _sessions: &Sessions) -> Result<Value, String> {
    let conn: ConnectionFields = serde_json::from_value(
        params.get("connection").cloned().unwrap_or_else(|| params.clone()),
    )
    .map_err(|e| format!("Invalid connection params: {}", e))?;
    let url = endpoint_url(&conn);
    let (session, handle) = open_session(&conn).await?;
    // Probe with a root browse to confirm the session is usable.
    let _ = session
        .browse(
            &[BrowseDescription {
                node_id: NodeId::new(0, 84), // Root folder
                browse_direction: BrowseDirection::Forward,
                reference_type_id: NodeId::new(0, 33), // HierarchicalReferences
                include_subtypes: true,
                node_class_mask: 0,
                result_mask: 0xFFFFFFFF,
            }],
            1000,
            None,
        )
        .await;
    handle.abort(); // close the probe connection
    Ok(json!({
        "success": true,
        "message": format!("Connected to OPC UA endpoint {}.", url),
        "endpoint": url
    }))
}

async fn handle_connect(params: &Value, sessions: &Sessions) -> Result<Value, String> {
    let conn: ConnectionFields = serde_json::from_value(
        params.get("connection").cloned().unwrap_or_else(|| params.clone()),
    )
    .map_err(|e| format!("Invalid connection params: {}", e))?;
    let url = endpoint_url(&conn);
    // If already connected, drop the previous session first.
    {
        let mut guard = sessions.lock().unwrap();
        if let Some(prev) = guard.remove(&conn.id) {
            prev._loop_handle.abort();
        }
    }
    let (session, handle) = open_session(&conn).await?;
    sessions.lock().unwrap().insert(
        conn.id.clone(),
        ConnSession {
            session,
            _loop_handle: handle,
        },
    );
    Ok(json!({ "success": true, "connected": true, "id": conn.id, "endpoint": url }))
}

async fn handle_disconnect(params: &Value, sessions: &Sessions) -> Result<Value, String> {
    let id = extract_conn_id(params).ok_or_else(|| "Missing connection id".to_string())?;
    let removed = {
        let mut guard = sessions.lock().unwrap();
        guard.remove(&id).map(|c| {
            c._loop_handle.abort();
            true
        })
    };
    Ok(json!({ "success": true, "disconnected": removed.is_some(), "id": id }))
}

async fn handle_root(_params: &Value, _sessions: &Sessions) -> Result<Value, String> {
    // Standard OPC UA address-space roots.
    Ok(json!({
        "roots": [
            {"nodeId":"i=85","browseName":"Objects","displayName":"Objects","nodeClass":"Object"},
            {"nodeId":"i=86","browseName":"Types","displayName":"Types","nodeClass":"Object"},
            {"nodeId":"i=87","browseName":"Views","displayName":"Views","nodeClass":"Object"}
        ]
    }))
}

async fn handle_browse(params: &Value, sessions: &Sessions) -> Result<Value, String> {
    let conn_id = extract_conn_id(params).ok_or_else(|| "Missing connection id".to_string())?;
    let node_id_str = params
        .get("nodeId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing nodeId".to_string())?;
    let session = clone_session(&conn_id, sessions)?;
    let node_id =
        NodeId::from_str(node_id_str).map_err(|e| format!("Invalid nodeId '{}': {}", node_id_str, e))?;

    let desc = BrowseDescription {
        node_id,
        browse_direction: BrowseDirection::Forward,
        reference_type_id: NodeId::new(0, 33), // HierarchicalReferences
        include_subtypes: true,
        node_class_mask: 0,
        result_mask: 0xFFFFFFFF,
    };

    let mut refs: Vec<ReferenceDescription> = Vec::new();
    let mut cont: Vec<ByteString> = Vec::new();
    {
        let results = session
            .browse(&[desc.clone()], 1000, None)
            .await
            .map_err(|e| format!("Browse failed: {}", e))?;
        for r in &results {
            if let Some(list) = &r.references {
                refs.extend(list.iter().cloned());
            }
            if !r.continuation_point.is_null() {
                cont.push(r.continuation_point.clone());
            }
        }
    }
    while !cont.is_empty() {
        let next = session
            .browse_next(false, &cont)
            .await
            .map_err(|e| format!("BrowseNext failed: {}", e))?;
        cont = Vec::new();
        for r in &next {
            if let Some(list) = &r.references {
                refs.extend(list.iter().cloned());
            }
            if !r.continuation_point.is_null() {
                cont.push(r.continuation_point.clone());
            }
        }
    }

    let children: Vec<Value> = refs
        .iter()
        .map(|rd| {
            json!({
                "nodeId": rd.node_id.node_id.to_string(),
                "browseName": rd.browse_name.name.as_ref(),
                "displayName": rd.display_name.text.as_ref(),
                "nodeClass": node_class_name(rd.node_class),
                "referenceTypeId": rd.reference_type_id.to_string(),
                "hasChildren": null
            })
        })
        .collect();

    Ok(json!({ "nodeId": node_id_str, "children": children }))
}

async fn handle_read(params: &Value, sessions: &Sessions) -> Result<Value, String> {
    let conn_id = extract_conn_id(params).ok_or_else(|| "Missing connection id".to_string())?;
    let node_id_str = params
        .get("nodeId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing nodeId".to_string())?;
    let session = clone_session(&conn_id, sessions)?;
    let node_id =
        NodeId::from_str(node_id_str).map_err(|e| format!("Invalid nodeId '{}': {}", node_id_str, e))?;

    let read_id: ReadValueId = node_id.into();
    let values = session
        .read(&[read_id], TimestampsToReturn::Both, 0.0)
        .await
        .map_err(|e| format!("Read failed: {}", e))?;

    let dv = values.into_iter().next();
    match dv {
        Some(dv) => {
            let (data_type, value) = match &dv.value {
                Some(v) => variant_to_json(v),
                None => ("<null>".into(), Value::Null),
            };
            let status = dv.status.map(|s| s.to_string());
            let timestamp = dv.source_timestamp.map(|t| t.to_string());
            Ok(json!({
                "nodeId": node_id_str,
                "dataType": data_type,
                "value": value,
                "status": status,
                "sourceTimestamp": timestamp
            }))
        }
        None => Err("Read returned no value".to_string()),
    }
}

/// Batch-read several nodes in a single round trip — used by the watch-list poller so
/// N monitored points cost one request instead of N.
async fn handle_read_many(params: &Value, sessions: &Sessions) -> Result<Value, String> {
    let conn_id = extract_conn_id(params).ok_or_else(|| "Missing connection id".to_string())?;
    let raw = params
        .get("nodeIds")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Missing nodeIds array".to_string())?;

    // Accept both ["ns=2;s=A"] and [{"nodeId":"ns=2;s=A"}].
    let mut ids: Vec<String> = Vec::new();
    for item in raw {
        match item {
            Value::String(s) => ids.push(s.clone()),
            Value::Object(o) => {
                if let Some(s) = o.get("nodeId").and_then(|v| v.as_str()) {
                    ids.push(s.to_string());
                }
            }
            _ => {}
        }
    }
    if ids.is_empty() {
        return Ok(json!({ "values": [] }));
    }

    const MAX_NODES: usize = 500;
    if ids.len() > MAX_NODES {
        return Err(format!(
            "Too many nodeIds ({}); limit is {}",
            ids.len(),
            MAX_NODES
        ));
    }

    let session = clone_session(&conn_id, sessions)?;

    let mut parsed: Vec<(String, NodeId)> = Vec::with_capacity(ids.len());
    for s in ids {
        let nid = NodeId::from_str(&s).map_err(|e| format!("Invalid nodeId '{}': {}", s, e))?;
        parsed.push((s, nid));
    }

    let read_ids: Vec<ReadValueId> = parsed.iter().map(|(_, n)| n.clone().into()).collect();
    let values = session
        .read(&read_ids, TimestampsToReturn::Both, 0.0)
        .await
        .map_err(|e| format!("Read failed: {}", e))?;

    let out: Vec<Value> = parsed
        .into_iter()
        .zip(values.into_iter())
        .map(|((node_id, _), dv)| {
            let (data_type, value) = match &dv.value {
                Some(v) => variant_to_json(v),
                None => ("<null>".into(), Value::Null),
            };
            json!({
                "nodeId": node_id,
                "dataType": data_type,
                "value": value,
                "status": dv.status.map(|s| s.to_string()),
                "sourceTimestamp": dv.source_timestamp.map(|t| t.to_string())
            })
        })
        .collect();

    Ok(json!({ "values": out }))
}

async fn handle_write(params: &Value, sessions: &Sessions) -> Result<Value, String> {
    let conn_id = extract_conn_id(params).ok_or_else(|| "Missing connection id".to_string())?;
    let node_id_str = params
        .get("nodeId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing nodeId".to_string())?;
    let data_type = params
        .get("dataType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing dataType".to_string())?;
    let value = params
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing value".to_string())?;

    let session = clone_session(&conn_id, sessions)?;
    let node_id =
        NodeId::from_str(node_id_str).map_err(|e| format!("Invalid nodeId '{}': {}", node_id_str, e))?;
    let variant = parse_variant(data_type, value)?;

    let wv = WriteValue {
        node_id,
        attribute_id: AttributeId::Value as u32,
        index_range: NumericRange::None,
        value: DataValue::value_only(variant),
    };
    let statuses = session
        .write(&[wv])
        .await
        .map_err(|e| format!("Write failed: {}", e))?;
    let status = statuses
        .into_iter()
        .next()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    Ok(json!({ "nodeId": node_id_str, "status": status }))
}

// ---------------------------------------------------------------------------
// JSON-RPC dispatch loop
// ---------------------------------------------------------------------------

async fn dispatch(method: &str, params: &Value, sessions: &Sessions) -> Result<Value, String> {
    match method {
        "plugin/initialize" => handle_initialize(params),
        "connection/test" => handle_test(params, sessions).await,
        "connection/connect" => handle_connect(params, sessions).await,
        "connection/disconnect" => handle_disconnect(params, sessions).await,
        "opcua/root" => handle_root(params, sessions).await,
        "opcua/browse" => handle_browse(params, sessions).await,
        "opcua/read" => handle_read(params, sessions).await,
        "opcua/readMany" => handle_read_many(params, sessions).await,
        "opcua/write" => handle_write(params, sessions).await,
        _ => Err(format!("Method not found: {}", method)),
    }
}

async fn handle_line(line: String, sessions: Sessions) {
    let request: Value = match serde_json::from_str(&line) {
        Ok(v) => v,
        Err(e) => {
            log_err(format!("Malformed JSON request: {}", e));
            return;
        }
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = match request.get("method").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => {
            write_response(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32600, "message": "Invalid Request: missing method"}
            }));
            return;
        }
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    let result = dispatch(&method, &params, &sessions).await;
    match result {
        Ok(r) => write_response(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": r
        })),
        Err(msg) => {
            if msg.starts_with("Method not found") {
                write_response(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": msg}
                }));
            } else {
                write_response(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32000, "message": msg}
                }));
            }
        }
    }
}

/// Serializes stdout writes.
///
/// Requests are handled concurrently (one task per line), so two responses could
/// otherwise interleave *between* the JSON write and the newline write and corrupt
/// the line framing the host relies on. One lock + one `write_all` per response
/// guarantees each response lands as a single intact line.
static STDOUT_LOCK: Mutex<()> = Mutex::new(());

fn write_response(resp: &Value) {
    let mut line = resp.to_string();
    line.push('\n');
    let _guard = STDOUT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(line.as_bytes());
    let _ = stdout.flush();
}

#[tokio::main]
async fn main() {
    eprintln!(
        "[dbx-opcua-sidecar] starting (id={}, version={})",
        PLUGIN_ID, PLUGIN_VERSION
    );
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let sessions = sessions.clone();
                let owned = trimmed.to_string();
                tokio::spawn(async move {
                    handle_line(owned, sessions).await;
                });
            }
            Err(e) => {
                log_err(format!("stdin read error: {}", e));
                break;
            }
        }
    }
    eprintln!("[dbx-opcua-sidecar] stdin closed, shutting down");
}
