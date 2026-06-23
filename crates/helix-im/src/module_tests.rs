//! `module.rs` 单测外提 sibling（structure-gate「测试外提」范本，对齐 `state_tests.rs`）。
//! A4 hello 握手回放测试（直接断言 `pub(crate)` state）。`#[path]` 子模块：`super` = `module`。

use super::*;
use helix_core::effect::TransportId;
use helix_core::tick::InboundBytes;

fn module_with_channel() -> ImModule {
    let mut m = ImModule::new(ImConfig::default());
    m.register_channel(crate::state::test_channel_id(1), 0);
    m
}

fn hello_frame(conn_id: &str) -> Tick {
    let body = serde_json::json!({
        "action": "hello",
        "data": { "connectionId": conn_id, "serverVersion": "x", "pingInterval": 30000u64 }
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&body).unwrap().into()))
}

fn has_http(sink: &EffectSink) -> bool {
    sink.as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Http { .. }))
}

/// Connected → conn 变 Connecting，且不产 resync Http（现网 Go 握手前 resync 会 401）。
#[test]
fn connected_sets_connecting_no_resync() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    assert_eq!(m.state.conn, ConnState::Connecting);
    assert!(!has_http(&sink), "握手前不得 resync");
}

/// hello Inbound → Connected + emit im:connection:established + resync Http + connectionId 存。
#[test]
fn hello_completes_handshake_and_resyncs() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    sink.clear();
    m.handle(&hello_frame("CONN_ABC"), 1001, &mut sink).unwrap();
    assert_eq!(m.state.conn, ConnState::Connected);
    assert_eq!(m.state.connection_id.as_deref(), Some("CONN_ABC"));
    assert!(
        sink.as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "应 emit im:connection:established"
    );
    assert!(has_http(&sink), "hello 后应 resync");
}

/// hello 后构造的 sync Http headers 含 ("connectionId", id)。
#[test]
fn sync_http_carries_connection_id_header() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    sink.clear();
    m.handle(&hello_frame("CONN_XYZ"), 1001, &mut sink).unwrap();
    let req = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } => Some(req),
            _ => None,
        })
        .expect("应有 resync Http");
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "connectionId" && v == "CONN_XYZ"),
        "sync Http 须带 connectionId 身份头"
    );
}

/// 顺序不变性：先 Connected 后 hello，终态 Connected + connectionId 已存。
#[test]
fn order_connected_then_hello_terminal_connected() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    m.handle(&hello_frame("C1"), 1001, &mut sink).unwrap();
    assert_eq!(m.state.conn, ConnState::Connected);
    assert_eq!(m.state.connection_id.as_deref(), Some("C1"));
}
