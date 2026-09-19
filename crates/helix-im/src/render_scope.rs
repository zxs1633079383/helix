//! 用户级持久化与公司级渲染的独立边界。只筛 Emit，不丢弃补水或持久化操作。
use crate::module::ImConfig;
use helix_core::effect::{SqlValue, StorageOp};
use helix_core::{Correlation, Effect, EffectSink, Tick};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct RenderScope {
    // 归属只来自持久 channel 行；消息作者不是本地库所属用户。
    channels: HashMap<String, (String, String)>,
    pending: HashMap<Correlation, Vec<(String, String, String)>>,
}

#[derive(Deserialize)]
struct Envelope<'a> {
    event: &'a str,
    #[serde(borrow)]
    data: &'a serde_json::value::RawValue,
}

#[derive(Deserialize)]
struct CorrelatedRead<'a> {
    req_id: Option<&'a str>,
}

impl RenderScope {
    /// 按需离线查询可以早于启动扫描完成，仍从这次真实本地读回建立同一归属索引。
    pub(crate) fn observe_rows(&mut self, bytes: &[u8]) {
        if let Ok(rows) = serde_json::from_slice::<Vec<serde_json::Value>>(bytes) {
            for row in rows {
                self.observe_channel(&row);
            }
        }
    }

    /// 冷启动从当前用户库的持久 channel 行恢复归属；未知/缺失归属不能猜为当前公司。
    pub(crate) fn observe_channel(&mut self, row: &serde_json::Value) {
        if let (Some(id), Some(team), Some(user)) = (
            row.get("id").and_then(serde_json::Value::as_str),
            row.get("team_id").and_then(serde_json::Value::as_str),
            row.get("user_id").and_then(serde_json::Value::as_str),
        ) {
            self.channels.insert(id.into(), (team.into(), user.into()));
        }
    }

    /// 归属变更与数据一样只在对应 PersistOk 后生效，失败/重复回执不改变授权索引。
    pub(crate) fn observe_reply(&mut self, tick: &Tick) {
        if let Tick::PortReply { corr, outcome } = tick {
            if let Some(changes) = self.pending.remove(corr) {
                if matches!(outcome, helix_core::tick::PortOutcome::Ok(_)) {
                    for (id, team, user) in changes {
                        self.channels.insert(id, (team, user));
                    }
                }
            }
        }
    }

    /// 暂存归属写入，既不把写入意图当提交，也不影响任何存储 Effect。
    pub(crate) fn stage_ops(&mut self, corr: Correlation, ops: &[StorageOp]) {
        let mut changes = Vec::new();
        for op in ops {
            if let StorageOp::BatchUpdate(spec) = op {
                if spec.table == "channel" && spec.key_col == "id" {
                    let team = spec.patch.iter().find_map(|(key, value)| match value {
                        SqlValue::Text(team) if key == "team_id" => Some(team),
                        _ => None,
                    });
                    if let Some(team) = team {
                        for id in &spec.key_vals {
                            if let SqlValue::Text(id) = id {
                                if let Some((_, user)) = self.channels.get(id) {
                                    // hot-path-audit: ignore - 低频频道归属变更的三个标量需跨 PersistOk 持有，非消息热路径。
                                    changes.push((
                                        id.clone(),   // hot-path-audit: ignore
                                        team.clone(), // hot-path-audit: ignore
                                        user.clone(), // hot-path-audit: ignore
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            let StorageOp::BatchUpsert(spec) = op else {
                continue;
            };
            if spec.table != "channel" {
                continue;
            }
            for row in &spec.rows {
                let text = |key| {
                    row.iter().find_map(|(k, v)| match v {
                        SqlValue::Text(value) if k == key => Some(value.as_str()),
                        _ => None,
                    })
                };
                if let (Some(id), Some(team), Some(user)) =
                    (text("id"), text("team_id"), text("user_id"))
                {
                    if !self.channels.contains_key(id)
                        || !spec.exclude_from_update.contains(&"team_id")
                    {
                        changes.push((id.into(), team.into(), user.into()));
                    }
                }
            }
        }
        if !changes.is_empty() {
            self.pending.insert(corr, changes);
        }
    }

    /// 用户和公司都必须与可信 RuntimeAuth 一致；未知频道 fail-closed。
    pub(crate) fn visible(&self, config: &ImConfig, channel: &str) -> bool {
        !config.auth_user_id.is_empty()
            && !config.company_id.is_empty()
            && self.channels.get(channel).is_some_and(|(team, user)| {
                team == &config.company_id && user == &config.auth_user_id
            })
    }

    /// 补水读回核对频道真实归属，不使用当前 UI 公司替代资源公司。
    pub(crate) fn company_for(&self, channel: &str) -> Option<&str> {
        self.channels.get(channel).map(|(team, _)| team.as_str())
    }

    /// 只有完整的非空 team/owner 组合才算已知频道归属；空稀疏行仍需补水。
    pub(crate) fn has_trusted_channel_scope(&self, config: &ImConfig, channel: &str) -> bool {
        self.channels.get(channel).is_some_and(|(team, user)| {
            !team.is_empty() && user == &config.auth_user_id && !config.auth_user_id.is_empty()
        })
    }

    /// 当前用户/公司的频道集合只用于本地水位，不裁剪后台跨公司同步游标。
    pub(crate) fn scoped_channel_ids(&self, config: &ImConfig) -> Vec<crate::state::ChannelId> {
        let mut ids: Vec<_> = self
            .channels
            .keys()
            .filter(|id| self.visible(config, id))
            .filter_map(|id| crate::state::ChannelId::from_str(id))
            .collect();
        ids.sort_unstable();
        ids
    }

    /// 所有运行平台共用的出口；正常事件零重编码，借用式扫描不复制消息内容。
    pub(crate) fn guard_effects(&mut self, config: &ImConfig, start: usize, out: &mut EffectSink) {
        for effect in &out.as_slice()[start..] {
            match effect {
                Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops } => {
                    self.stage_ops(*corr, ops)
                }
                _ => {}
            }
        }
        out.retain_mut_from(start, |effect| {
            let Effect::Emit { event } = effect else {
                return true;
            };
            let Ok(envelope) = serde_json::from_slice::<Envelope<'_>>(event.0.as_ref()) else {
                return false;
            };
            let channel_row = matches!(
                envelope.event,
                "im:channel:created"
                    | "im:channel:update"
                    | "im:channel:increment"
                    | "im:channel:topic-created"
            );
            let mut visitor = ScopeVisitor {
                scope: self,
                config,
                row: channel_row,
                valid: true,
                seen: false,
            };
            let parsed = (&mut visitor)
                .deserialize(&mut serde_json::Deserializer::from_str(envelope.data.get()))
                .is_ok();
            let requires_channel = (envelope.event.starts_with("im:post:")
                || envelope.event.starts_with("im:timeline:")
                || (envelope.event.starts_with("im:channel:")
                    && envelope.event != "im:channel:list"))
                && !envelope.event.ends_with("failed");
            if parsed && visitor.valid && (!requires_channel || visitor.seen) {
                return true;
            }
            tracing::debug!(
                event = envelope.event,
                reason = "render_scope_mismatch",
                "suppressed out-of-scope render projection"
            );
            // 读请求必须结算，但不能带回任何外公司 body。广播则完全不发送。
            if envelope.event == "im:read:result" {
                if let Ok(CorrelatedRead {
                    req_id: Some(req_id),
                }) = serde_json::from_str(envelope.data.get())
                {
                    *effect = crate::read_relay::emit_read_error(req_id, "RENDER_SCOPE_MISMATCH");
                    return true;
                }
            }
            false
        });
    }
}

struct ScopeVisitor<'a> {
    scope: &'a RenderScope,
    config: &'a ImConfig,
    row: bool,
    valid: bool,
    seen: bool,
}

impl<'de> DeserializeSeed<'de> for &mut ScopeVisitor<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for &mut ScopeVisitor<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("render projection")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let row = self.row;
        while let Some(key) = map.next_key::<&str>()? {
            if matches!(key, "channelId" | "channel_id" | "parentChannelId") || (row && key == "id")
            {
                let id = map.next_value::<Option<&str>>()?;
                if let Some(id) = id.filter(|id| !id.is_empty()) {
                    self.seen = true;
                    self.valid &= self.scope.visible(self.config, id);
                }
            } else if key == "id" {
                // 列表里的频道行通常只携带 id；不要把成员/消息自身 id 当成频道。
                let raw = map.next_value::<&serde_json::value::RawValue>()?;
                let id = serde_json::from_str::<&str>(raw.get()).ok();
                if let Some(id) = id.filter(|id| self.scope.channels.contains_key(*id)) {
                    self.seen = true;
                    self.valid &= self.scope.visible(self.config, id);
                }
            } else if matches!(key, "teamId" | "team_id") {
                let entity_team = map.next_value::<Option<&str>>()?;
                if row {
                    self.valid &= entity_team == Some(self.config.company_id.as_str());
                }
            } else if matches!(key, "content" | "props" | "metadata" | "extra") {
                // 不把业务文本或任意扩展 JSON 中的同名字段误判为路由身份。
                map.next_value::<IgnoredAny>()?;
            } else {
                self.row = matches!(key, "channel" | "dialog" | "snapshot");
                map.next_value_seed(&mut *self)?;
            }
        }
        // 任意成员 id 也可能是 Id26；只有明确 channel 行能把 teamId 当频道归属。
        // byIds 等 map-key 协议在对应 producer 按频道 key 审查，不在这里猜字段语义。
        self.row = row;
        Ok(())
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        while seq.next_element_seed(&mut *self)?.is_some() {}
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}
