//! im:__bus__ 信封守护（M2 Tauri 接缝① 横切约束）。
//!
//! `event_sink.rs` 的 [`to_bus_envelope`](crate::to_bus_envelope) 把 `DomainEventBytes`
//! （`{event, data}`）转成总线信封 `{channel, payload}`。本文件在其之上叠加 **P6 接缝①
//! 两条横切约束**，保持 `event_sink.rs` 不增长（≤321 基线）+ 单一职责分离：
//!
//! 1. **channel 自环守护**：真 `app.emit` 出口的 channel 必须 `im:` 开头**且非** `im:__`。
//!    - `im:` 前缀：CoreEventManager dispatch 的命名空间约定（现网 event_bus.rs）。
//!    - 非 `im:__`：`im:__bus__` 是**总线本身**的 Tauri event 名；若业务事件 channel 也叫
//!      `im:__*` 会被前端 dispatcher 当总线信封再解一层 → **自环**。守护把这类非法 channel
//!      标记出来（grep 守护 + 运行时降级），driver 出口据此**不 emit**（防自环风暴）。
//!
//! 2. **数组 payload `{items:[]}` 包装**（全 Phase 横切，rv-correctness M4）：若 `payload.data`
//!    本身是 JSON 数组（投影/列表事件），包装成 `{"items":[...]}`——前端 dispatcher 统一按对象
//!    解构，不必为「裸数组 vs 对象」两套分支。已是对象的 data 原样保留。
//!
//! ## 为什么不直接改 `to_bus_envelope`
//!
//! `to_bus_envelope` 是 host-cli（无窗口，靠计数验证）与 Tauri（真 emit）**共用**的纯转换；
//! 自环守护是**真 emit 出口**特有的约束（host-cli 不 emit 无此风险）。分离让 host-cli 路径零负担，
//! Tauri 出口（`app.emit` 前）调 [`guard_bus_envelope`] 一次即可。

use helix_core::effect::DomainEventBytes;

use crate::event_sink::to_bus_envelope;

/// 总线本身的 Tauri event 名前缀——业务事件 channel 撞此前缀 = 自环（见模块头注）。
const BUS_SELF_PREFIX: &str = "im:__";
/// 业务事件 channel 命名空间前缀（CoreEventManager dispatch 约定）。
const IM_PREFIX: &str = "im:";

/// 守护结果：channel 合法性 + 待 emit 的信封（data 数组已 `{items:[]}` 包装）。
#[derive(Debug, Clone, PartialEq)]
pub struct GuardedEnvelope {
    /// 业务 channel 名（如 `im:post:received`）。
    pub channel: String,
    /// 待 `app.emit(BUS_CHANNEL, &envelope)` 的信封 `{channel, payload}`。
    pub envelope: serde_json::Value,
    /// channel 是否合法（`im:` 开头且非 `im:__`）。非法 → driver 出口**不 emit**（防自环）。
    pub channel_ok: bool,
}

/// 把 `DomainEventBytes` 转成守护后的总线信封（真 emit 出口前调一次）。
///
/// 步骤：① `to_bus_envelope` 拿 `{channel, payload}`；② channel 合法性判定（`im:` 前缀 +
/// 非 `im:__`）；③ `payload.data` 若为数组则 `{items:[...]}` 包装写回。零信任：非法 JSON /
/// 缺字段经 `to_bus_envelope` 已回退空 channel（`channel_ok=false`）。
pub fn guard_bus_envelope(ev: &DomainEventBytes) -> GuardedEnvelope {
    let mut envelope = to_bus_envelope(ev);
    let channel = envelope
        .get("channel")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let channel_ok = is_emittable_channel(&channel);

    // data 数组 → {items:[]} 包装（横切 M4）。payload 是 {event,data}；只动 data。
    // 链式 get_mut：任一层缺失（非法 payload）→ 整体 None 不包装（零信任，单一假分支即可覆盖）。
    if let Some(data) = envelope
        .get_mut("payload")
        .and_then(|p| p.get_mut("data"))
        .filter(|d| d.is_array())
    {
        let arr = data.take();
        *data = serde_json::json!({ "items": arr });
    }

    GuardedEnvelope {
        channel,
        envelope,
        channel_ok,
    }
}

/// channel 是否可 emit：`im:` 开头**且非** `im:__`（防自环）。
///
/// 纯函数（grep 守护可单测）：`im:__bus__` / `im:__internal` 等总线/内部前缀一律拒。
pub fn is_emittable_channel(channel: &str) -> bool {
    channel.starts_with(IM_PREFIX) && !channel.starts_with(BUS_SELF_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn ev(json: serde_json::Value) -> DomainEventBytes {
        DomainEventBytes(Bytes::from(serde_json::to_vec(&json).unwrap()))
    }

    /// 合法业务 channel（im: 前缀非 im:__）→ channel_ok=true，data 对象原样。
    #[test]
    fn object_data_passes_through_with_ok_channel() {
        let g = guard_bus_envelope(&ev(serde_json::json!({
            "event": "im:post:received",
            "data": { "channel_id": "ch_1", "event_seq": 5 }
        })));
        assert_eq!(g.channel, "im:post:received");
        assert!(g.channel_ok);
        // 对象 data 不被 {items} 包装。
        assert_eq!(
            g.envelope["payload"]["data"],
            serde_json::json!({ "channel_id": "ch_1", "event_seq": 5 })
        );
    }

    /// 数组 data → {items:[...]} 包装（横切 M4）。
    #[test]
    fn array_data_wrapped_in_items() {
        let g = guard_bus_envelope(&ev(serde_json::json!({
            "event": "im:messages:query_result",
            "data": [ {"id": "m1"}, {"id": "m2"} ]
        })));
        assert!(g.channel_ok);
        assert_eq!(
            g.envelope["payload"]["data"],
            serde_json::json!({ "items": [ {"id": "m1"}, {"id": "m2"} ] }),
            "数组 data 必须 {{items:[]}} 包装，前端 dispatcher 统一对象解构"
        );
    }

    /// 自环守护：channel 撞 im:__ 前缀（总线本身）→ channel_ok=false（driver 不 emit）。
    #[test]
    fn bus_self_prefix_channel_rejected() {
        let g = guard_bus_envelope(&ev(serde_json::json!({
            "event": "im:__bus__",
            "data": {}
        })));
        assert_eq!(g.channel, "im:__bus__");
        assert!(!g.channel_ok, "im:__ 前缀 channel 必须被拒（防自环）");
    }

    /// 非 im: 前缀（外部命名空间误入）→ channel_ok=false。
    #[test]
    fn non_im_prefix_channel_rejected() {
        let g = guard_bus_envelope(&ev(serde_json::json!({
            "event": "store:setItem",
            "data": {}
        })));
        assert!(!g.channel_ok);
    }

    /// 零信任：非法 JSON → 空 channel + channel_ok=false（不 panic）。
    #[test]
    fn invalid_json_yields_empty_channel_not_ok() {
        let g = guard_bus_envelope(&DomainEventBytes(Bytes::from_static(b"not json{")));
        assert_eq!(g.channel, "");
        assert!(!g.channel_ok);
    }

    /// is_emittable_channel 纯函数边界表。
    #[test]
    fn emittable_channel_boundary_table() {
        assert!(is_emittable_channel("im:post:received"));
        assert!(is_emittable_channel("im:channel:update"));
        assert!(!is_emittable_channel("im:__bus__"));
        assert!(!is_emittable_channel("im:__internal"));
        assert!(!is_emittable_channel("store:get"));
        assert!(!is_emittable_channel(""));
    }
}
