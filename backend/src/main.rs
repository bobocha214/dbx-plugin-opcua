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
const PLUGIN_ID: &str = "io.github.yuzhi.opcua";
const PLUGIN_VERSION: &str = "0.2.0";
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

    // `connect_to_matching_endpoint` only *prepares* the session — the actual secure-channel /
    // CreateSession handshake runs inside the event loop, which is spawned just below. Waiting
    // here is what makes `connection/connect` an honest success signal: without it the first
    // read/write issued right after connecting races the transport and fails with
    // BadNotConnected ("Unable to send request, transport is not connected").
    let handle = event_loop.spawn();
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, session.wait_for_connection()).await {
        Ok(true) => Ok((session, handle)),
        Ok(false) => {
            handle.abort();
            Err(format!("Cannot connect to {}: session closed during the handshake", url))
        }
        Err(_) => {
            handle.abort();
            Err(format!(
                "Cannot connect to {}: the OPC UA handshake did not finish within {}s",
                url,
                HANDSHAKE_TIMEOUT.as_secs()
            ))
        }
    }
}

/// How long a connection may take to complete its handshake.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Fetch the session for a connection and make sure its transport is usable.
///
/// Reconnecting is asynchronous, so a request that arrives while the session is being
/// re-established would otherwise fail with a bare BadNotConnected.
async fn ready_session(conn_id: &str, sessions: &Sessions) -> Result<Arc<Session>, String> {
    let session = clone_session(conn_id, sessions)?;
    // Returns immediately when the session is already connected (watch channel semantics).
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, session.wait_for_connection()).await {
        Ok(true) => Ok(session),
        Ok(false) => Err(format!(
            "Connection '{}' was closed. Open it in DBX again, then retry.",
            conn_id
        )),
        Err(_) => Err(format!(
            "Connection '{}' is not ready: the session did not reconnect within {}s. \
             If the network dropped, reopen the connection in DBX.",
            conn_id,
            HANDSHAKE_TIMEOUT.as_secs()
        )),
    }
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

/// Data types this sidecar can build a Variant for. Everything else (DateTime, Guid,
/// ExtensionObject, arrays, ...) is rejected with an explicit message instead of being
/// silently coerced into something the server will reject with BadTypeMismatch.
const WRITABLE_TYPES: [&str; 12] = [
    "Boolean", "SByte", "Byte", "Int16", "UInt16", "Int32", "UInt32", "Int64", "UInt64", "Float",
    "Double", "String",
];

/// Map a namespace-0 type NodeId (`i=1`..`i=25`) to its type name.
fn builtin_type_name(node_id: &NodeId) -> Option<&'static str> {
    if node_id.namespace != 0 {
        return None;
    }
    let id = match &node_id.identifier {
        Identifier::Numeric(id) => *id,
        _ => return None,
    };
    Some(match id {
        1 => "Boolean",
        2 => "SByte",
        3 => "Byte",
        4 => "Int16",
        5 => "UInt16",
        6 => "Int32",
        7 => "UInt32",
        8 => "Int64",
        9 => "UInt64",
        10 => "Float",
        11 => "Double",
        12 => "String",
        13 => "DateTime",
        14 => "Guid",
        15 => "ByteString",
        16 => "XmlElement",
        17 => "NodeId",
        18 => "ExpandedNodeId",
        19 => "StatusCode",
        20 => "QualifiedName",
        21 => "LocalizedText",
        22 => "ExtensionObject",
        23 => "DataValue",
        24 => "Variant",
        25 => "DiagnosticInfo",
        _ => return None,
    })
}

/// What the server says about a node, read in one round trip before writing.
///
/// Without this the client has to guess the type, and a wrong guess is reported by the
/// server as a bare `BadTypeMismatch` that is indistinguishable from a broken write.
struct NodeWriteInfo {
    /// Concrete builtin type reported by the server; `None` for abstract (BaseDataType,
    /// Number, ...) or custom structure types.
    server_type: Option<String>,
    /// -1 = scalar, -3 = scalar or 1-dim, 0 = 1-dim array, >0 = n-dim array.
    value_rank: i32,
    /// Bit 1 (0x02) means CurrentWrite is allowed.
    user_access_level: Option<u8>,
    data_type_id: Option<String>,
}

const VALUE_RANK_UNKNOWN: i32 = -2;

async fn probe_node(session: &Session, node_id: &NodeId) -> NodeWriteInfo {
    let attr = |a: AttributeId| ReadValueId {
        node_id: node_id.clone(),
        attribute_id: a as u32,
        index_range: NumericRange::None,
        data_encoding: QualifiedName::null(),
    };
    let reads = vec![
        attr(AttributeId::DataType),
        attr(AttributeId::ValueRank),
        attr(AttributeId::UserAccessLevel),
    ];

    let mut info = NodeWriteInfo {
        server_type: None,
        value_rank: VALUE_RANK_UNKNOWN,
        user_access_level: None,
        data_type_id: None,
    };

    // A failed probe is not fatal: the caller may still pass an explicit type.
    let values = match session.read(&reads, TimestampsToReturn::Neither, 0.0).await {
        Ok(v) => v,
        Err(_) => return info,
    };

    if let Some(Some(Variant::NodeId(nid))) = values.first().map(|v| v.value.clone()) {
        info.data_type_id = Some(nid.to_string());
        info.server_type = builtin_type_name(&nid).map(str::to_string);
    }
    if let Some(Some(Variant::Int32(rank))) = values.get(1).map(|v| v.value.clone()) {
        info.value_rank = rank;
    }
    if let Some(Some(Variant::Byte(level))) = values.get(2).map(|v| v.value.clone()) {
        info.user_access_level = Some(level);
    }

    info
}

/// Strip separators a human might type and normalise a single decimal comma (`1,5`).
fn normalize_number(raw: &str) -> String {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if s.matches(',').count() == 1 && !s.contains('.') {
        s.replace(',', ".")
    } else {
        s
    }
}

fn num_err(raw: &str, data_type: &str, e: &dyn std::fmt::Display) -> String {
    let detail = e.to_string();
    let reason = if detail.contains("too large") {
        "超出该类型的最大可表示值".to_string()
    } else if detail.contains("too small") || detail.contains("negative") {
        "超出该类型的最小可表示值".to_string()
    } else if detail.contains("invalid digit") || detail.contains("float literal") {
        "包含无法解析的字符".to_string()
    } else {
        detail
    };
    format!("值 '{}' 不是合法的 {}（{}）", raw, data_type, reason)
}

fn parse_num<T>(raw: &str, data_type: &str) -> Result<T, String>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    normalize_number(raw)
        .parse::<T>()
        .map_err(|e| num_err(raw, data_type, &e))
}

/// Parse a scalar value string into a Variant for writing.
fn parse_variant(data_type: &str, raw: &str) -> Result<Variant, String> {
    let value = raw.trim();
    match data_type {
        "Boolean" => match value.to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "on" | "1" => Ok(Variant::Boolean(true)),
            "false" | "f" | "no" | "n" | "off" | "0" => Ok(Variant::Boolean(false)),
            _ => Err(format!(
                "值 '{}' 不是合法的 Boolean（可用 true/false、1/0、yes/no、on/off）",
                raw
            )),
        },
        "SByte" => parse_num::<i8>(value, data_type).map(Variant::SByte),
        "Byte" => parse_num::<u8>(value, data_type).map(Variant::Byte),
        "Int16" => parse_num::<i16>(value, data_type).map(Variant::Int16),
        "UInt16" => parse_num::<u16>(value, data_type).map(Variant::UInt16),
        "Int32" => parse_num::<i32>(value, data_type).map(Variant::Int32),
        "UInt32" => parse_num::<u32>(value, data_type).map(Variant::UInt32),
        "Int64" => parse_num::<i64>(value, data_type).map(Variant::Int64),
        "UInt64" => parse_num::<u64>(value, data_type).map(Variant::UInt64),
        "Float" => parse_num::<f32>(value, data_type).map(Variant::Float),
        "Double" => parse_num::<f64>(value, data_type).map(Variant::Double),
        "String" => Ok(Variant::String(UAString::from(raw.to_string()))),
        other => Err(format!(
            "暂不支持写入 {} 类型（支持 {}）",
            other,
            WRITABLE_TYPES.join("/")
        )),
    }
}

/// Readable explanation for a failed Write service result.
fn write_status_hint(code: &StatusCode, data_type: &str) -> String {
    match code.sub_code().name() {
        "BadTypeMismatch" => format!("值类型与节点要求的 {} 不匹配", data_type),
        "BadNotWritable" => "该节点不可写（服务器报告 NotWritable，常见于只读点位或 PLC 侧未开放写权限）".into(),
        "BadUserAccessDenied" => "权限不足：当前用户或安全策略不允许写入该节点".into(),
        "BadNodeIdUnknown" => "节点在该服务器的地址空间中不存在".into(),
        "BadOutOfRange" => "写入值超出节点允许的范围".into(),
        "BadWriteNotSupported" => "该节点不支持写入".into(),
        "BadNotConnected" => "会话已断开，请在 DBX 中重新打开连接".into(),
        "BadTooManyOperations" => "服务器拒绝了本次写入请求".into(),
        _ => format!("服务器返回 {}", code.sub_code().name()),
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
    let session = ready_session(&conn_id, sessions).await?;
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
    let session = ready_session(&conn_id, sessions).await?;
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

    let session = ready_session(&conn_id, sessions).await?;

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

/// `value` may arrive as a JSON string (the UI always sends one) or as a raw number /
/// boolean, so this method stays usable from scripts and other clients.
fn scalar_to_string(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

async fn handle_write(params: &Value, sessions: &Sessions) -> Result<Value, String> {
    let conn_id = extract_conn_id(params).ok_or_else(|| "Missing connection id".to_string())?;
    let node_id_str = params
        .get("nodeId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing nodeId".to_string())?;
    let value = scalar_to_string(params.get("value"))
        .ok_or_else(|| "Missing value (expected a string, number or boolean)".to_string())?;
    let requested = params
        .get("dataType")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Auto")
        .to_string();

    let session = ready_session(&conn_id, sessions).await?;
    let node_id =
        NodeId::from_str(node_id_str).map_err(|e| format!("Invalid nodeId '{}': {}", node_id_str, e))?;

    // Ask the server what this node actually is before guessing a type.
    let info = probe_node(&session, &node_id).await;

    // Arrays and matrices need an array Variant; fail with the real reason instead of a
    // generic BadTypeMismatch.
    if info.value_rank == 0 || info.value_rank > 0 {
        return Err(format!(
            "节点 {} 是数组（ValueRank={}），当前只支持写入标量值",
            node_id_str, info.value_rank
        ));
    }

    // The server-reported type wins: writing an incompatible Variant is always rejected,
    // so a stale hand-picked type from the UI must not override it.
    let (data_type, type_source) = match &info.server_type {
        Some(t) => (t.clone(), "server"),
        None => {
            if requested.eq_ignore_ascii_case("Auto") {
                let what = match &info.data_type_id {
                    Some(id) => format!("DataType 为 {}（抽象类型或自定义结构）", id),
                    None => "DataType 属性读取失败".to_string(),
                };
                return Err(format!(
                    "无法自动推断写入类型：节点 {} 的 {}。请在右侧手动选择数据类型后再写入。",
                    node_id_str, what
                ));
            }
            (requested.clone(), "request")
        }
    };

    let variant = parse_variant(&data_type, &value)?;

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
    let code = statuses
        .into_iter()
        .next()
        .unwrap_or(StatusCode::BadNothingToDo);
    let success = code.is_good();

    // A good status code is still returned as `success: false` when the server says Bad —
    // the UI must never render a rejected write as a success.
    Ok(json!({
        "nodeId": node_id_str,
        "success": success,
        "status": code.to_string(),
        "statusCode": code.bits(),
        "message": if success { Value::Null } else { Value::String(write_status_hint(&code, &data_type)) },
        "dataType": data_type,
        "dataTypeSource": type_source,
        "requestedDataType": requested,
        "valueRank": info.value_rank,
        "userAccessLevel": info.user_access_level,
    }))
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
