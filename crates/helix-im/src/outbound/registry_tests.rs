//! `registry.rs` 单测外提 sibling（structure-gate「测试外提」范本，对齐 `state_tests.rs`）。
//!
//! `#[path]` 子模块：`super` = `registry` 模块，复用其 `pub(crate)` 项（`build_registry`/`OutboundCommand`
//! /`require_str` 等）。把测试搬出 `registry.rs` 主体压回行数闸 ≤300（含第二网关族新增 gateway 测试）。

use super::*;
use serde_json::json;

// 测试桩命令：build 直回固定 (path, body)（不涉真现网 body）。
struct StubCommand {
    name: &'static str,
}
impl OutboundCommand for StubCommand {
    fn name(&self) -> &'static str {
        self.name
    }
    fn build(&self, _args: &Value) -> Result<(&'static str, Value), ImError> {
        Ok(("stub/path", json!({ "ok": true })))
    }
}

static STUB_A: StubCommand = StubCommand { name: "im_stub_a" };
static STUB_DUP: StubCommand = StubCommand { name: "im_stub_a" };
static STUB_MISMATCH: StubCommand = StubCommand {
    name: "handler_name",
};

fn reg(name: &'static str, cmd: &'static dyn OutboundCommand) -> OutboundRegistration {
    OutboundRegistration { name, command: cmd }
}

/// build_registry 正常路径 + name/handler 一致校验通过。
#[test]
fn build_registry_ok() {
    let entries: &'static [OutboundRegistration] = Box::leak(Box::new([reg("im_stub_a", &STUB_A)]));
    let map = build_registry(entries).expect("should build");
    assert!(map.contains_key("im_stub_a"));
}

/// 注册 name 与 command.name() 错配 → InvalidWsHandlerRegistration。
#[test]
fn build_registry_rejects_name_mismatch() {
    let entries: &'static [OutboundRegistration] =
        Box::leak(Box::new([reg("wrong_name", &STUB_MISMATCH)]));
    match build_registry(entries) {
        Err(ImError::InvalidWsHandlerRegistration {
            registered,
            handler,
        }) => {
            assert_eq!(registered, "wrong_name");
            assert_eq!(handler, "handler_name");
        }
        Err(e) => panic!("expected InvalidWsHandlerRegistration, got {e:?}"),
        Ok(_) => panic!("expected InvalidWsHandlerRegistration, got Ok"),
    }
}

/// 重复命令名 → DuplicateWsAction。
#[test]
fn build_registry_rejects_duplicate() {
    let entries: &'static [OutboundRegistration] = Box::leak(Box::new([
        reg("im_stub_a", &STUB_A),
        reg("im_stub_a", &STUB_DUP),
    ]));
    match build_registry(entries) {
        Err(ImError::DuplicateWsAction(name)) => assert_eq!(name, "im_stub_a"),
        Err(e) => panic!("expected DuplicateWsAction, got {e:?}"),
        Ok(_) => panic!("expected DuplicateWsAction, got Ok"),
    }
}

/// require_str：命中 / 缺失 / 空串 三路。
#[test]
fn require_str_paths() {
    let args = json!({ "k": "v", "empty": "" });
    assert_eq!(require_str(&args, "k", "cmd").unwrap(), "v");
    assert!(require_str(&args, "missing", "cmd").is_err());
    assert!(require_str(&args, "empty", "cmd").is_err());
}

/// is_outbound：真实注册表（distributed_slice 全集）命中既有命令、不命中未知。
#[test]
fn is_outbound_hits_registered() {
    assert!(is_outbound("im_revoke"));
    assert!(is_outbound("im_urgent_post"));
    assert!(!is_outbound("not_a_command"));
}

/// handle_outbound：正常路径产单条 Http（覆盖 http_post）+ 未知命令 Err + 坏 payload Err。
#[test]
fn handle_outbound_dispatch_and_errors() {
    let corr = Correlation::from_raw(1);
    let payload = serde_json::to_vec(&json!({
        "post_id": "p",
        "req_id": "mrc-run:G-01:req-1"
    }))
    .unwrap();
    let effects = handle_outbound(
        "im_revoke",
        &payload,
        "http://h/api/cses",
        "http://h",
        Some("c1"),
        corr,
    )
    .expect("revoke should dispatch");
    match &effects[0] {
        Effect::Http { req, .. } => {
            assert_eq!(req.url, "http://h/api/cses/posts/revoke");
            assert!(req
                .headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "application/json"));
            assert!(req
                .headers
                .iter()
                .any(|(k, v)| k == "Cses-Track-Id" && v == "mrc-run:G-01:req-1"));
        }
        other => panic!("expected Http, got {other:?}"),
    }
    // 未知命令 → Err
    assert!(handle_outbound(
        "bogus",
        &payload,
        "http://h/api/cses",
        "http://h",
        None,
        corr
    )
    .is_err());
    // 坏 JSON payload → Err（不 panic）
    assert!(handle_outbound(
        "im_revoke",
        b"not-json",
        "http://h/api/cses",
        "http://h",
        None,
        corr
    )
    .is_err());
}

// ── 第二网关（spec06 方案 A）：vote/score 走 default base，既有命令仍走 IM base（回归）─────

/// vote 命令 `gateway()==Default` 且 handle_outbound 用 **default base**（无 `/api/cses`）拼 url。
/// 断言 url 含 `localhost:3399`（第二网关）且 **不含** `/api/cses`（不串到 IM 网关）。
#[test]
fn vote_command_uses_default_gateway_base() {
    assert!(is_outbound("im_vote_read"), "im_vote_read 应已注册");
    let corr = Correlation::from_raw(7);
    let payload = serde_json::to_vec(&json!({ "id": "v1" })).unwrap();
    let effects = handle_outbound(
        "im_vote_read",
        &payload,
        "http://h:8065/api/cses", // IM 网关（不该被用）
        "http://localhost:3399",  // 第二网关（应被用）
        Some("c1"),
        corr,
    )
    .expect("vote_read should dispatch");
    match &effects[0] {
        Effect::Http { req, .. } => {
            assert_eq!(req.url, "http://localhost:3399/vote/readVote");
            assert!(
                !req.url.contains("/api/cses"),
                "vote 走第二网关，url 不该含 /api/cses，实际={}",
                req.url
            );
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

/// 回归：既有 IM 命令 `gateway()==Im` 仍走 **api_base_url**（IM 网关，含 `/api/cses`），
/// 第二 base 不串扰。证「88 既有命令零改动」。
#[test]
fn existing_im_command_stays_on_im_gateway() {
    let corr = Correlation::from_raw(8);
    let payload = serde_json::to_vec(&json!({ "post_id": "p" })).unwrap();
    let effects = handle_outbound(
        "im_revoke",
        &payload,
        "http://h:8065/api/cses",
        "http://localhost:3399", // 第二网关存在但 im_revoke 不该用它
        None,
        corr,
    )
    .expect("revoke should dispatch");
    match &effects[0] {
        Effect::Http { req, .. } => {
            assert_eq!(req.url, "http://h:8065/api/cses/posts/revoke");
            assert!(
                !req.url.contains("localhost:3399"),
                "IM 命令不该走第二网关，实际={}",
                req.url
            );
        }
        other => panic!("expected Http, got {other:?}"),
    }
}
