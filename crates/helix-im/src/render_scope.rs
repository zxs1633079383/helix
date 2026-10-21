//! 用户级持久化与成员频道渲染的边界。跨公司可见，其他账号及未知频道仍拒绝。
use crate::module::ImConfig;
use helix_core::effect::{SqlValue, StorageOp};
use helix_core::{Correlation, Effect, EffectSink, Tick};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct RenderScope {
    // 归属只来自持久 channel 行；消息作者不是本地库所属用户。
    channels: HashMap<String, (String, String)>,
    // 防止旧Get回包把已读回的新共享归属回退。
    channel_versions: HashMap<String, i64>,
    // 仅用于本轮已认证远端分页读；不写持久归属，发送该 req_id 后即消耗。
    authority_reads: HashMap<String, HashSet<String>>,
    pending: HashMap<Correlation, Vec<(String, String, String)>>,
    // 版本化写入可能被拒绝；精确读回后以durable归属纠正写入意图。
    pending_channel_reads: HashMap<Correlation, Vec<String>>,
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

#[derive(Deserialize)]
struct TimelineRead<'a> {
    #[serde(borrow)]
    body: TimelineBody<'a>,
}

#[derive(Deserialize)]
struct TimelineBody<'a> {
    operation: &'a str,
    #[serde(rename = "channelId")]
    channel_id: Option<&'a str>,
    #[serde(borrow)]
    messages: Option<&'a serde_json::value::RawValue>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopePath {
    Data,
    Body,
    Messages,
    MessageRow,
    Other,
}

impl RenderScope {
    /// O(Effect 数量 + 请求 JSON 长度) 路由已提交频道的 HTTP 租户；不改变账号/凭据或全局请求。
    pub(crate) fn scope_http_requests(
        &self,
        config: &ImConfig,
        tick: &Tick,
        start: usize,
        out: &mut EffectSink,
    ) {
        #[derive(Deserialize)]
        struct ChannelRequest<'a> {
            #[serde(default, alias = "channelId", borrow)]
            channel_id: Option<&'a str>,
        }
        let command_channel = match tick {
            Tick::Command(command) => {
                serde_json::from_slice::<ChannelRequest<'_>>(command.payload.as_ref())
                    .ok()
                    .and_then(|request| request.channel_id)
            }
            _ => None,
        };
        out.retain_mut_from(start, |effect| {
            let Effect::Http { req, .. } = effect else {
                return true;
            };
            // Java/OSS 与未确认的资源不继承群租户；Host 继续注入原登录身份。
            if config.api_base_url.is_empty()
                || !req
                    .url
                    .strip_prefix(config.api_base_url.trim_end_matches('/'))
                    .is_some_and(|suffix| suffix.starts_with('/'))
            {
                return true;
            }
            let channel = req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<ChannelRequest<'_>>(body.as_ref()).ok())
                .and_then(|request| request.channel_id)
                .or(command_channel);
            if let Some(channel) =
                channel.filter(|channel| self.has_trusted_channel_scope(config, channel))
            {
                if let Some(company) = self.company_for(channel) {
                    req.headers
                        .retain(|(name, _)| !name.eq_ignore_ascii_case("companyId"));
                    req.headers.push(("companyId".into(), company.into()));
                }
            }
            true
        });
    }

    /// 权威全局读已验证账号与企业后，允许该单次结果中的未缓存频道通过渲染围栏。
    pub(crate) fn authorize_read(&mut self, req_id: &str, channels: HashSet<String>) {
        self.authority_reads.insert(req_id.to_owned(), channels);
    }

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
            let version = row
                .get("snapshot_updated_at")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            if version < self.channel_versions.get(id).copied().unwrap_or(0) {
                return;
            }
            self.channel_versions.insert(id.into(), version);
            self.channels.insert(id.into(), (team.into(), user.into()));
        }
    }

    /// 归属变更与数据一样只在对应 PersistOk 后生效，失败/重复回执不改变授权索引。
    pub(crate) fn observe_reply(&mut self, tick: &Tick) {
        if let Tick::PortReply { corr, outcome } = tick {
            if let Some(ids) = self.pending_channel_reads.remove(corr) {
                if let helix_core::tick::PortOutcome::Ok(reply) = outcome {
                    if let Ok(rows) =
                        serde_json::from_slice::<Vec<serde_json::Value>>(reply.0.as_ref())
                    {
                        for row in rows {
                            if row
                                .get("id")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|id| ids.iter().any(|expected| expected == id))
                            {
                                self.observe_channel(&row);
                            }
                        }
                    }
                }
            }

            if let Some(changes) = self.pending.remove(corr) {
                if matches!(outcome, helix_core::tick::PortOutcome::Ok(_)) {
                    for (id, team, user) in changes {
                        self.channels.insert(id, (team, user));
                    }
                }
            }
        }
    }

    /// 仅设置/补水的冷路径精确读回登记归属；不在每条消息的Get上追加JSON解析。
    pub(crate) fn track_channel_read(&mut self, corr: Correlation, channel_id: &str) {
        self.pending_channel_reads
            .insert(corr, vec![channel_id.to_owned()]);
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

    /// 持久账户必须匹配；有效群租户不受登录公司限制，未知频道仍拒绝。
    pub(crate) fn visible(&self, config: &ImConfig, channel: &str) -> bool {
        !config.auth_user_id.is_empty()
            && self.channels.get(channel).is_some_and(|(team, user)| {
                crate::identity::AccountScope::new(&config.auth_user_id, &config.company_id)
                    .accepts_channel_tenant(Some(team))
                    && user == &config.auth_user_id
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

    /// 当前账号的成员频道集合用于本地水位，不裁剪跨公司同步游标。
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
            let authority_channels = if envelope.event == "im:read:result" {
                serde_json::from_str::<CorrelatedRead<'_>>(envelope.data.get())
                    .ok()
                    .and_then(|read| read.req_id)
                    .and_then(|req_id| self.authority_reads.remove(req_id))
            } else {
                None
            };
            let timeline_read = if envelope.event == "im:read:result" {
                serde_json::from_str::<TimelineRead<'_>>(envelope.data.get())
                    .ok()
                    .filter(|read| matches!(read.body.operation, "older" | "newer" | "locate"))
            } else {
                None
            };
            let mut visitor = ScopeVisitor {
                scope: self,
                authority_channels: authority_channels.as_ref(),
                config,
                row: channel_row,
                path: if timeline_read.is_some() {
                    ScopePath::Data
                } else {
                    ScopePath::Other
                },
                valid: timeline_read.as_ref().map_or(true, |read| {
                    read.body.channel_id.is_some_and(|id| !id.is_empty())
                        && read
                            .body
                            .messages
                            .is_some_and(|messages| messages.get().trim_start().starts_with('['))
                }),
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
            // 读请求必须结算，但不能带回未授权频道 body。广播则完全不发送。
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
    authority_channels: Option<&'a HashSet<String>>,
    config: &'a ImConfig,
    row: bool,
    path: ScopePath,
    valid: bool,
    seen: bool,
}

impl<'de> DeserializeSeed<'de> for &mut ScopeVisitor<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        if self.path == ScopePath::MessageRow {
            d.deserialize_map(self)
        } else {
            d.deserialize_any(self)
        }
    }
}

impl<'de> Visitor<'de> for &mut ScopeVisitor<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("render projection")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let row = self.row;
        let path = self.path;
        let mut message_route_seen = false;
        while let Some(key) = map.next_key::<&str>()? {
            if matches!(key, "channelId" | "channel_id" | "parentChannelId") || (row && key == "id")
            {
                let id = map.next_value::<Option<&str>>()?;
                if path == ScopePath::MessageRow && matches!(key, "channelId" | "channel_id") {
                    message_route_seen = true;
                    self.valid &= id.is_some_and(|id| !id.is_empty());
                }
                if let Some(id) = id.filter(|id| !id.is_empty()) {
                    self.seen = true;
                    self.valid &= self.authority_channels.is_some_and(|ids| ids.contains(id))
                        || self.scope.visible(self.config, id);
                }
            } else if key == "id" {
                // 列表里的频道行通常只携带 id；不要把成员/消息自身 id 当成频道。
                let raw = map.next_value::<&serde_json::value::RawValue>()?;
                let id = serde_json::from_str::<&str>(raw.get()).ok();
                if let Some(id) = id.filter(|id| self.scope.channels.contains_key(*id)) {
                    self.seen = true;
                    self.valid &= self.authority_channels.is_some_and(|ids| ids.contains(id))
                        || self.scope.visible(self.config, id);
                }
            } else if matches!(key, "teamId" | "team_id") {
                let entity_team = map.next_value::<Option<&str>>()?;
                if row {
                    self.valid &= crate::identity::AccountScope::new(
                        &self.config.auth_user_id,
                        &self.config.company_id,
                    )
                    .accepts_channel_tenant(entity_team);
                }
            } else if matches!(key, "content" | "props" | "metadata" | "extra" | "topic") {
                // topic 是父消息携带的引用，不是本次投影的频道归属；其频道可能尚未补水。
                // 主会话与消息自身的 channelId 仍逐项校验，话题详情由独立读取校验。
                // 不把业务文本或任意扩展 JSON 中的同名字段误判为路由身份。
                map.next_value::<IgnoredAny>()?;
            } else {
                self.row = matches!(key, "channel" | "dialog" | "snapshot");
                self.path = match (path, key) {
                    (ScopePath::Data, "body") => ScopePath::Body,
                    (ScopePath::Body, "messages") => ScopePath::Messages,
                    _ => ScopePath::Other,
                };
                map.next_value_seed(&mut *self)?;
                self.row = row;
                self.path = path;
            }
        }
        if path == ScopePath::MessageRow {
            self.valid &= message_route_seen;
        }
        // 任意成员 id 也可能是 Id26；只有明确 channel 行能把 teamId 当频道归属。
        // byIds 等 map-key 协议在对应 producer 按频道 key 审查，不在这里猜字段语义。
        self.row = row;
        self.path = path;
        Ok(())
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        let path = self.path;
        loop {
            self.path = if path == ScopePath::Messages {
                ScopePath::MessageRow
            } else {
                ScopePath::Other
            };
            let next = seq.next_element_seed(&mut *self)?;
            self.path = path;
            if next.is_none() {
                break;
            }
        }
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
