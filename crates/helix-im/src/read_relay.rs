//! spec06 缺陷A — 读族 outbound HTTP 响应回灌前端（request-response 回灌通道）。
//!
//! 读族命令（top20/getReplies/bookmark/announcement/user-misc 读/bot 读…）**无 WS 回声、HTTP
//! 响应体本身就是数据**——dispatch 对读命令注册 `CorrelationContext::OutboundReadReply{req_id}`，
//! HTTP 回报时本模块工厂把**后端响应体原样透传**回灌成 `im:read:result{req_id, body}`。前端 bridge
//! （仿 `helixQueryBridge`）`invoke(命令, {…, req_id})` 后等 `im:read:result{req_id}` resolve → 直渲。
//!
//! ## 不变量
//! - **透传**：后端返回什么吐什么，helix 不改业务数据（后端权威，HX-C005 不在热路径瞎序列化）。
//! - **不丢 req_id**：成功 `{req_id, body}` / 失败 `{req_id, error}`——失败也回灌让前端 reject，不挂起。
//! - **非投影**：body 形态随 endpoint 变（不冻结字段集，与 21 投影集分离，见 projection-schema §1.2）。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};

/// 读族 HTTP 成功回报 → emit `im:read:result{req_id, body}`（body = 后端响应体透传）。
///
/// `raw_body` = `unwrap_sync_envelope` 解出的**裸后端响应体字节**（ADR-007 信封 base64 已剥）。
/// body 优先按 JSON 原样嵌入（前端零二次 parse）；非 JSON（防御性，读族理论不会）→ lossy UTF-8
/// 字符串占位（保 req_id 关联，不 panic）。
pub fn emit_read_result(req_id: &str, raw_body: &[u8]) -> Effect {
    let body: serde_json::Value = serde_json::from_slice(raw_body)
        .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(raw_body).into_owned()));
    emit_read(serde_json::json!({ "req_id": req_id, "body": body }))
}

/// 读族 HTTP 失败 / 信封畸形 → emit `im:read:result{req_id, error}`（不丢 req_id，前端 reject）。
///
/// `error` 仅诊断字符串（不含敏感数据，边界零信任），让前端等待的 promise reject 而非永久挂起超时。
pub fn emit_read_error(req_id: &str, error: &str) -> Effect {
    emit_read(serde_json::json!({ "req_id": req_id, "error": error }))
}

/// 内部：把 `data` 包成 `im:read:result` 信封 Effect::Emit（成功/失败共用 channel）。
fn emit_read(data: serde_json::Value) -> Effect {
    let payload = serde_json::json!({ "event": "im:read:result", "data": data });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("emit_read: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit_json(e: Effect) -> serde_json::Value {
        match e {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).unwrap(),
            _ => panic!("expected Emit"),
        }
    }

    #[test]
    fn read_result_passes_body_through_verbatim() {
        let raw = serde_json::to_vec(&serde_json::json!({ "status": "SUCCESS", "data": [1, 2] }))
            .unwrap();
        let v = emit_json(emit_read_result("rq-1", &raw));
        assert_eq!(v["event"], "im:read:result");
        assert_eq!(v["data"]["req_id"], "rq-1");
        assert_eq!(v["data"]["body"]["status"], "SUCCESS");
        assert_eq!(v["data"]["body"]["data"][1], 2);
        assert!(v["data"].get("error").is_none());
    }

    #[test]
    fn read_result_non_json_body_degrades_to_string() {
        let v = emit_json(emit_read_result("rq-2", b"plain text not json"));
        assert_eq!(v["data"]["body"], "plain text not json");
    }

    #[test]
    fn read_error_keeps_req_id_and_no_body() {
        let v = emit_json(emit_read_error("rq-3", "http request failed"));
        assert_eq!(v["event"], "im:read:result");
        assert_eq!(v["data"]["req_id"], "rq-3");
        assert_eq!(v["data"]["error"], "http request failed");
        assert!(v["data"].get("body").is_none());
    }
}
