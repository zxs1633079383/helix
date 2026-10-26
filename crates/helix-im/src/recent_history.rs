//! 本机最近搜索/转发/表情；身份只取运行时，读改写串行且 PersistOk 后才返回新状态。
//! 入口：Tick::Command(im_recent_history)，kind=search|forward|emoji-send|emoji-reaction，action=read|record|remove|clear|import。
//! 运行时身份生成 scope；写请求必须回传 read 得到的 scope，targets 只含 type/id。表情用独立 kind，最新使用移到头部，最多 32 条。
//! 本地确定性命令：ScopedGet → 可选 BatchUpsert → PersistOk → im:read:result。
//! 校验失败返回 ModuleError；存储失败/身份变化发出带 req_id 的读取错误并释放队列。
//! corr 在统一 PortReply 分发中消费，重复/迟到回包不再执行；模块销毁释放在途状态。
//! HTTP、WS、远程业务失败和重连：N/A；等待超时及取消由 Host 查询生命周期处理。
//! 已验证：公开 Module 单测及真实 HostStorage SQLite；PRE UI 尚未验证。
//! ponytail: 最多三十二条的低频偏好使用整行 JSON；不用于消息热路径或无界集合。
use crate::{error::ImError, event::MessageV3Event, module::ImModule, state::CorrelationContext};
use helix_core::{
    effect::{Effect, ScopedGetSpec, SqlValue, StorageOp, UpsertSpec},
    tick::PortOutcome,
    EffectSink,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};

pub const COMMAND: &str = "im_recent_history";
const TABLE: &str = "im_recent_history";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub kind: String,
    pub action: String,
    pub scope: Option<String>,
    #[serde(default)]
    pub targets: Vec<Target>,
    pub req_id: String,
}
#[derive(Debug, Default)]
pub struct HistoryState {
    pub busy: bool,
    pub queue: VecDeque<Command>,
}
fn scope(module: &ImModule) -> Result<String, ImError> {
    let c = &module.config;
    if c.auth_user_id.is_empty() || c.api_base_url.is_empty() {
        return Err(ImError::Parse(
            "recent history requires runtime identity".into(),
        ));
    }
    Ok(json!([c.api_base_url, c.company_id, c.auth_user_id]).to_string())
}
fn normalize(targets: Vec<Target>, limit: usize) -> Vec<Target> {
    let mut seen = HashSet::new();
    targets
        .into_iter()
        .filter(|t| seen.insert((t.kind.clone(), t.id.clone())))
        .take(limit)
        .collect()
}
fn validate(c: &Command) -> Result<(), ImError> {
    if !["search", "forward", "emoji-send", "emoji-reaction"].contains(&c.kind.as_str())
        || !["read", "record", "remove", "clear", "import"].contains(&c.action.as_str())
        || c.req_id.is_empty()
        || c.targets.len() > 100
        || c.targets.iter().any(|t| {
            t.id.trim().is_empty()
                || t.id.len() > 512
                || (if c.kind.starts_with("emoji-") {
                    t.kind != "emoji"
                } else {
                    !["user", "channel"].contains(&t.kind.as_str())
                })
                || (c.kind == "forward" && t.kind != "channel")
        })
        || (c.action == "record" && c.targets.is_empty())
        || (c.action == "remove" && c.targets.len() != 1)
        || (["read", "clear"].contains(&c.action.as_str()) && !c.targets.is_empty())
    {
        return Err(ImError::Parse("invalid recent history command".into()));
    }
    Ok(())
}
pub fn handle(module: &mut ImModule, payload: &[u8], out: &mut EffectSink) -> Result<(), ImError> {
    let mut c: Command =
        serde_json::from_slice(payload).map_err(|e| ImError::Parse(e.to_string()))?;
    validate(&c)?;
    let current = scope(module)?;
    if c.action != "read" && c.scope.as_ref() != Some(&current) {
        return Err(ImError::Parse("recent history scope changed".into()));
    }
    c.scope = Some(current);
    if module.state.recent_history.queue.len() >= 64 {
        return Err(ImError::Parse("recent history queue full".into()));
    }
    module.state.recent_history.queue.push_back(c);
    start_next(module, out);
    Ok(())
}
fn start_next(module: &mut ImModule, out: &mut EffectSink) {
    if module.state.recent_history.busy {
        return;
    }
    let Some(c) = module.state.recent_history.queue.pop_front() else {
        return;
    };
    module.state.recent_history.busy = true;
    let corr = module.alloc_corr_internal();
    let op = StorageOp::ScopedGet(ScopedGetSpec {
        table: TABLE,
        scope_col: "scope",
        scope_val: SqlValue::Text(c.scope.clone().unwrap_or_default()),
        key_col: "kind",
        key_val: SqlValue::Text(c.kind.clone()),
    });
    module.state.corr_map.insert(
        corr,
        CorrelationContext::RecentHistoryRead {
            command: Box::new(c),
        },
    );
    out.push(Effect::Persist {
        corr,
        ops: vec![op],
    });
}
fn finish(module: &mut ImModule, out: &mut EffectSink) {
    module.state.recent_history.busy = false;
    start_next(module, out);
}
fn fail(module: &mut ImModule, c: &Command, out: &mut EffectSink) {
    out.push(crate::read_relay::emit_read_error(
        &c.req_id,
        "recent history storage or scope failure",
    ));
    finish(module, out);
}
fn text<'a>(row: &'a helix_core::effect::Row, col: &str) -> Option<&'a str> {
    row.iter().find_map(|(k, v)| match v {
        SqlValue::Text(s) if k == col => Some(s.as_str()),
        _ => None,
    })
}
fn emit(c: &Command, result: Value, out: &mut EffectSink) -> Result<(), ImError> {
    out.push(
        MessageV3Event::new(
            "im:read:result",
            json!({"req_id": c.req_id, "body": result}),
        )?
        .into_effect(),
    );
    Ok(())
}
pub fn read_reply(
    module: &mut ImModule,
    c: Box<Command>,
    outcome: &PortOutcome,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    if scope(module).ok().as_ref() != c.scope.as_ref() {
        fail(module, &c, out);
        return Ok(());
    }
    let PortOutcome::Ok(reply) = outcome else {
        fail(module, &c, out);
        return Ok(());
    };
    let Ok(rows) = helix_core::port_codec::rows_from_reply_bytes(&reply.0) else {
        fail(module, &c, out);
        return Ok(());
    };
    let row = rows.iter().find(|r| {
        text(r, "scope") == c.scope.as_deref() && text(r, "kind") == Some(c.kind.as_str())
    });
    let existing: Vec<Target> = match row {
        Some(r) => match text(r, "items").and_then(|s| serde_json::from_str(s).ok()) {
            Some(items) => items,
            None => {
                fail(module, &c, out);
                return Ok(());
            }
        },
        None => vec![],
    };
    let limit = if c.kind.starts_with("emoji-") { 32 } else { 10 };
    let items = match c.action.as_str() {
        "record" => normalize(c.targets.iter().cloned().chain(existing).collect(), limit),
        "remove" => existing
            .into_iter()
            .filter(|t| !c.targets.contains(t))
            .collect(),
        "clear" => vec![],
        "import" if row.is_none() => normalize(c.targets.clone(), limit),
        _ => existing,
    };
    let result = json!({"scope": c.scope, "items": items, "initialized": row.is_some() || c.action != "read"});
    if c.action == "read" || (c.action == "import" && row.is_some()) {
        emit(&c, result, out)?;
        finish(module, out);
        return Ok(());
    }
    let corr = module.alloc_corr_internal();
    let row = vec![
        (
            "scope".into(),
            SqlValue::Text(c.scope.clone().unwrap_or_default()),
        ),
        ("kind".into(), SqlValue::Text(c.kind.clone())),
        (
            "items".into(),
            SqlValue::Text(
                serde_json::to_string(&items).map_err(|e| ImError::Parse(e.to_string()))?,
            ),
        ),
    ];
    module.state.corr_map.insert(
        corr,
        CorrelationContext::RecentHistoryWrite { command: c, result },
    );
    out.push(Effect::Persist {
        corr,
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: TABLE,
            rows: vec![row],
            conflict_key: None,
            exclude_from_update: vec![],
            update_guard: None,
            version_column: None,
        })],
    });
    Ok(())
}
pub fn write_reply(
    module: &mut ImModule,
    c: Box<Command>,
    result: Value,
    outcome: &PortOutcome,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    if !matches!(outcome, PortOutcome::Ok(_)) || scope(module).ok().as_ref() != c.scope.as_ref() {
        fail(module, &c, out);
        return Ok(());
    }
    emit(&c, result, out)?;
    finish(module, out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::{
        effect::Row,
        tick::{AppCommand, PortError, ReplyBytes},
        Module, Tick,
    };
    use std::collections::HashMap;

    fn module() -> ImModule {
        ImModule::new(crate::module::ImConfig {
            auth_user_id: "actor".into(),
            company_id: "company".into(),
            api_base_url: "https://im.test".into(),
            ..Default::default()
        })
    }
    fn enqueue(m: &mut ImModule, sink: &mut EffectSink, action: &str, targets: Value) {
        let tick = Tick::Command(AppCommand::new(COMMAND, serde_json::to_vec(&json!({
            "kind":"search","action":action,"scope":scope(m).unwrap(),"targets":targets,"req_id":action
        })).unwrap()));
        assert!(m.accepts(&tick));
        m.handle(&tick, 0, sink).unwrap();
    }
    // 模拟通用 StorageOp 驱动，不复制业务规则；返回实际 rows codec。
    fn drain(
        m: &mut ImModule,
        sink: &mut EffectSink,
        db: &mut HashMap<(String, String), Row>,
        fail_write: bool,
    ) -> Vec<Value> {
        let mut effects = std::mem::take(sink);
        let mut results = vec![];
        while !effects.is_empty() {
            for effect in effects.as_slice() {
                match effect {
                    Effect::Persist { corr, ops } => {
                        let mut rows = vec![];
                        let mut failed = false;
                        for op in ops.clone() {
                            match op {
                                StorageOp::ScopedGet(spec) => {
                                    if let (SqlValue::Text(s), SqlValue::Text(k)) =
                                        (spec.scope_val, spec.key_val)
                                    {
                                        rows.extend(db.get(&(s, k)).cloned());
                                    } else {
                                        panic!("bad key")
                                    }
                                }
                                StorageOp::BatchUpsert(spec) => {
                                    if fail_write {
                                        failed = true;
                                        continue;
                                    }
                                    for row in spec.rows {
                                        db.insert(
                                            (
                                                text(&row, "scope").unwrap().into(),
                                                text(&row, "kind").unwrap().into(),
                                            ),
                                            row,
                                        );
                                    }
                                }
                                _ => panic!("unexpected op"),
                            }
                        }
                        let outcome = if failed {
                            PortOutcome::Err(PortError::Storage(1))
                        } else {
                            PortOutcome::Ok(ReplyBytes(
                                helix_core::port_codec::rows_to_reply_bytes(&rows),
                            ))
                        };
                        m.handle(
                            &Tick::PortReply {
                                corr: *corr,
                                outcome,
                            },
                            0,
                            sink,
                        )
                        .unwrap();
                    }
                    Effect::Emit { event } => {
                        results.push(serde_json::from_slice(&event.0).unwrap())
                    }
                    _ => panic!("local preference must not use network"),
                }
            }
            effects = std::mem::take(sink);
        }
        results
    }
    #[test]
    fn serial_record_limit_delete_clear_and_restart() {
        let mut m = module();
        let mut sink = EffectSink::new();
        let mut db = HashMap::new();
        let rows: Vec<_> = (0..12)
            .map(|i| json!({"type":"user","id":i.to_string()}))
            .collect();
        enqueue(&mut m, &mut sink, "record", json!(rows));
        enqueue(
            &mut m,
            &mut sink,
            "record",
            json!([{"type":"user","id":"5"}]),
        );
        assert_eq!(sink.as_slice().len(), 1);
        let results = drain(&mut m, &mut sink, &mut db, false);
        let last = &results.last().unwrap()["data"]["body"]["items"];
        assert_eq!(last.as_array().unwrap().len(), 10);
        assert_eq!(last[0]["id"], "5");
        let mut m = module();
        enqueue(&mut m, &mut sink, "read", json!([]));
        assert_eq!(
            drain(&mut m, &mut sink, &mut db, false)[0]["data"]["body"]["items"],
            *last
        );
        enqueue(
            &mut m,
            &mut sink,
            "remove",
            json!([{"type":"user","id":"5"}]),
        );
        assert_eq!(
            drain(&mut m, &mut sink, &mut db, false)[0]["data"]["body"]["items"]
                .as_array()
                .unwrap()
                .len(),
            9
        );
        enqueue(&mut m, &mut sink, "clear", json!([]));
        drain(&mut m, &mut sink, &mut db, false);
        enqueue(
            &mut m,
            &mut sink,
            "import",
            json!([{"type":"user","id":"old"}]),
        );
        assert_eq!(
            drain(&mut m, &mut sink, &mut db, false)[0]["data"]["body"]["items"],
            json!([])
        );
    }
    #[test]
    fn failed_write_keeps_old_state_and_identity_isolation() {
        let mut m = module();
        let mut sink = EffectSink::new();
        let mut db = HashMap::new();
        enqueue(
            &mut m,
            &mut sink,
            "record",
            json!([{"type":"user","id":"old"}]),
        );
        drain(&mut m, &mut sink, &mut db, false);
        enqueue(&mut m, &mut sink, "clear", json!([]));
        drain(&mut m, &mut sink, &mut db, true);
        enqueue(&mut m, &mut sink, "read", json!([]));
        assert_eq!(
            drain(&mut m, &mut sink, &mut db, false)[0]["data"]["body"]["items"][0]["id"],
            "old"
        );
        let old_scope = scope(&m).unwrap();
        m.config.auth_user_id = "other".into();
        let payload = serde_json::to_vec(
            &json!({"kind":"search","action":"clear","scope":old_scope,"req_id":"bad"}),
        )
        .unwrap();
        assert!(handle(&mut m, &payload, &mut sink).is_err());
        enqueue(&mut m, &mut sink, "read", json!([]));
        assert_eq!(
            drain(&mut m, &mut sink, &mut db, false)[0]["data"]["body"]["items"],
            json!([])
        );
        let payload = br#"{"kind":"search","action":"read","req_id":"x","account_id":"actor"}"#;
        assert!(handle(&mut m, payload, &mut sink).is_err());
    }
    #[test]
    fn emoji_history_recency_is_not_frequency_and_send_reaction_are_isolated() {
        let mut m = module();
        let mut sink = EffectSink::new();
        let mut db = HashMap::new();
        for (kind, id) in [
            ("emoji-send", "heart"),
            ("emoji-send", "heart"),
            ("emoji-send", "joy"),
            ("emoji-reaction", "+1"),
            ("emoji-send", "heart"),
        ] {
            let tick = Tick::Command(AppCommand::new(
                COMMAND,
                serde_json::to_vec(&json!({
                    "kind":kind,"action":"record","scope":scope(&m).unwrap(),
                    "targets":[{"type":"emoji","id":id}],"req_id":id
                }))
                .unwrap(),
            ));
            m.handle(&tick, 100, &mut sink).unwrap();
            let result = drain(&mut m, &mut sink, &mut db, false);
            assert_eq!(result[0]["data"]["body"]["items"][0]["id"], id);
        }
        let mut m = module();
        for (kind, expected) in [
            ("emoji-send", vec!["heart", "joy"]),
            ("emoji-reaction", vec!["+1"]),
        ] {
            let tick = Tick::Command(AppCommand::new(
                COMMAND,
                serde_json::to_vec(&json!({
                    "kind":kind,"action":"read","req_id":"read"
                }))
                .unwrap(),
            ));
            m.handle(&tick, 200, &mut sink).unwrap();
            let result = drain(&mut m, &mut sink, &mut db, false);
            let ids: Vec<_> = result[0]["data"]["body"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap())
                .collect();
            assert_eq!(ids, expected);
            assert!(m.state.corr_map.is_empty());
        }
    }

    #[test]
    fn emoji_limit_and_invalid_cross_kind_targets_fail_closed() {
        let mut m = module();
        let mut sink = EffectSink::new();
        let mut db = HashMap::new();
        let targets: Vec<_> = (0..40)
            .map(|i| json!({"type":"emoji","id":format!("emoji-{i}")}))
            .collect();
        let payload = serde_json::to_vec(&json!({"kind":"emoji-send","action":"record", "scope":scope(&m).unwrap(),"targets":targets,"req_id":"limit"})).unwrap();
        m.handle(
            &Tick::Command(AppCommand::new(COMMAND, payload)),
            0,
            &mut sink,
        )
        .unwrap();
        let result = drain(&mut m, &mut sink, &mut db, false);
        assert_eq!(
            result[0]["data"]["body"]["items"].as_array().unwrap().len(),
            32
        );
        for (kind, target_type) in [
            ("search", "emoji"),
            ("emoji-send", "user"),
            ("emoji-reaction", "channel"),
        ] {
            let payload = serde_json::to_vec(&json!({"kind":kind,"action":"record", "scope":scope(&m).unwrap(),"targets":[{"type":target_type,"id":"x"}],"req_id":"invalid"})).unwrap();
            assert!(m
                .handle(
                    &Tick::Command(AppCommand::new(COMMAND, payload)),
                    0,
                    &mut sink
                )
                .is_err());
            assert!(sink.is_empty());
            assert!(m.state.corr_map.is_empty());
        }
    }
}
