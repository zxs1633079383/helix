//! 父群话题全集的单写队列：服务端观察版本排序，集合/数量同事务，PersistOk 才发投影。
use crate::{
    error::ImError,
    module::ImModule,
    state::{ChannelId, CorrelationContext, ImState},
};
use helix_core::effect::{BatchUpdateSpec, SqlValue, StorageOp, UpsertGuard, UpsertSpec};
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, Effect, EffectSink};
use std::collections::HashSet;

pub(crate) struct Inventory {
    pub version: i64,
    pub ids: Vec<ChannelId>,
    pub ops: Vec<StorageOp>,
    pub request: Option<String>,
    pub attempts: u8,
    pub loaded_at: i64,
}
#[derive(Default)]
pub(crate) struct Lane {
    active: Option<Inventory>,
    queued: Option<Inventory>,
    committed: i64,
}

/// 严格解析权威全集，版本通过十进制字符串传输，不接受重复/越界ID。
pub(crate) fn parse(value: &serde_json::Value) -> Result<Inventory, ImError> {
    let version = value["version"]
        .as_str()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| ImError::Parse("invalid topic inventory version".into()))?;
    let raw = value["channelIds"]
        .as_array()
        .filter(|ids| ids.len() <= 2000)
        .ok_or_else(|| ImError::Parse("invalid topic inventory ids".into()))?;
    let mut seen = HashSet::new();
    let mut ids = Vec::with_capacity(raw.len());
    for value in raw {
        let id = value
            .as_str()
            .and_then(ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("invalid topic inventory id".into()))?;
        if !seen.insert(id) {
            return Err(ImError::Parse("duplicate topic inventory id".into()));
        };
        ids.push(id)
    }
    Ok(Inventory {
        version,
        ids,
        ops: vec![],
        request: None,
        attempts: 0,
        loaded_at: 0,
    })
}

/// 每父群至多一个在途和一个最新候选，旧/重复观察不会造成无界排队。
pub(crate) fn enqueue(
    state: &mut ImState,
    parent: ChannelId,
    item: Inventory,
    corr: Correlation,
    out: &mut EffectSink,
) {
    let lane = state.topic_inventories.entry(parent).or_default();
    if item.version <= lane.committed {
        return;
    }
    if let Some(active) = &lane.active {
        if item.version <= active.version
            || lane
                .queued
                .as_ref()
                .is_some_and(|q| q.version > item.version)
        {
            return;
        }
        lane.queued = Some(item);
        return;
    }
    lane.active = Some(item);
    begin(state, parent, corr, out);
}
/// 开始一条清单写入的 durable revision 读取。
fn begin(state: &mut ImState, parent: ChannelId, corr: Correlation, out: &mut EffectSink) {
    state
        .corr_map
        .insert(corr, CorrelationContext::TopicInventoryRead { parent });
    out.push(Effect::Persist {
        corr,
        ops: vec![crate::acl::to_effect::get_channel_row_op(parent)],
    });
}

impl ImModule {
    /// 接收清单元数据回读；CAS guard 把跨写队列竞争变成整笔回滚。
    pub(crate) fn topic_inventory_read(
        &mut self,
        parent: ChannelId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let row = match outcome {
            PortOutcome::Ok(reply) => serde_json::from_slice::<Vec<serde_json::Value>>(&reply.0)
                .ok()
                .and_then(|rows| rows.into_iter().find(|r| r["id"] == parent.as_str())),
            _ => None,
        };
        let Some(row) = row else {
            return self.topic_inventory_finish(parent, false, out);
        };
        let version = row["topic_inventory_version"].as_i64().unwrap_or(0);
        let Some(lane) = self.state.topic_inventories.get_mut(&parent) else {
            return Ok(());
        };
        lane.committed = lane.committed.max(version);
        let Some(item) = lane.active.as_mut() else {
            return Ok(());
        };
        if item.version <= version {
            return self.topic_inventory_finish(parent, false, out);
        }
        let mut ops = item.ops.clone();
        let mut values = vec![
            ("id".into(), SqlValue::Text(parent.as_str().into())),
            (
                "topic_inventory_version".into(),
                SqlValue::Integer(item.version),
            ),
            (
                "topic_inventory_ids".into(),
                SqlValue::Text(
                    serde_json::to_string(
                        &item.ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
                    )
                    .map_err(|e| ImError::Parse(e.to_string()))?,
                ),
            ),
            (
                "thread_count".into(),
                SqlValue::Integer(item.ids.len() as i64),
            ),
        ];
        if item.request.is_some() {
            values.push((
                "subtopics_loaded_at".into(),
                SqlValue::Integer(item.loaded_at),
            ))
        }
        ops.insert(
            0,
            StorageOp::BatchUpsert(UpsertSpec {
                table: "channel",
                rows: vec![values],
                conflict_key: Some("id"),
                exclude_from_update: vec![],
                version_column: None,
                update_guard: Some(UpsertGuard {
                    column: "topic_inventory_version",
                    expected: SqlValue::Integer(version),
                }),
            }),
        );
        ops.push(StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "root_id",
            key_vals: vec![SqlValue::Text(parent.as_str().into())],
            patch: vec![("is_remove".into(), SqlValue::Integer(1))],
        }));
        if !item.ids.is_empty() {
            ops.push(StorageOp::BatchUpdate(BatchUpdateSpec {
                table: "channel",
                key_col: "id",
                key_vals: item
                    .ids
                    .iter()
                    .map(|id| SqlValue::Text(id.as_str().into()))
                    .collect(),
                patch: vec![("is_remove".into(), SqlValue::Integer(0))],
            }))
        }
        let corr = self.alloc_corr_internal();
        self.state
            .corr_map
            .insert(corr, CorrelationContext::TopicInventoryCommit { parent });
        out.push(Effect::PersistAtomic { corr, ops });
        Ok(())
    }
    /// 只有成功持久化且未被更高观察覆盖才发布数量/完成；继续有界队列。
    pub(crate) fn topic_inventory_finish(
        &mut self,
        parent: ChannelId,
        success: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(lane) = self.state.topic_inventories.get_mut(&parent) else {
            return Ok(());
        };
        let Some(mut item) = lane.active.take() else {
            return Ok(());
        };
        if !success && item.attempts < 2 && item.version > lane.committed && lane.queued.is_none() {
            item.attempts += 1;
            lane.active = Some(item);
            let corr = self.alloc_corr_internal();
            begin(&mut self.state, parent, corr, out);
            return Ok(());
        }
        if success {
            lane.committed = lane.committed.max(item.version)
        }
        let newer = lane
            .queued
            .as_ref()
            .is_some_and(|q| q.version > item.version);
        if success && !newer {
            out.push(crate::event::channel::update(serde_json::json!({"id":parent.as_str(),"threadCount":item.ids.len(),"topicChannelIds":item.ids.iter().map(|id|id.as_str()).collect::<Vec<_>>(),"topicInventoryVersion":item.version.to_string()}))?.into_effect());
            if let Some(request) = item.request.filter(|id| {
                self.state
                    .subtopic_requests
                    .get(&parent)
                    .is_none_or(|current| current == id)
            }) {
                out.push(
                    crate::event::channel_sync::subtopics_ready(
                        parent.as_str(),
                        Some(&request),
                        true,
                    )?
                    .into_effect(),
                );
            }
        }
        if let Some(next) = lane.queued.take() {
            let corr = self.alloc_corr_internal();
            enqueue(&mut self.state, parent, next, corr, out)
        }
        Ok(())
    }
}

/// 根群批量同步仅更新版本化父群投影，旧版本不覆盖实时写；不把集合变化拆成多个无守卫操作。
pub(crate) fn root_projection_op(
    parent: ChannelId,
    value: &serde_json::Value,
) -> Result<StorageOp, ImError> {
    let item = parse(value)?;
    Ok(StorageOp::BatchUpsert(UpsertSpec {
        table: "channel",
        rows: vec![vec![
            ("id".into(), SqlValue::Text(parent.as_str().into())),
            (
                "topic_inventory_version".into(),
                SqlValue::Integer(item.version),
            ),
            (
                "topic_inventory_ids".into(),
                SqlValue::Text(
                    serde_json::to_string(
                        &item.ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
                    )
                    .map_err(|e| ImError::Parse(e.to_string()))?,
                ),
            ),
            (
                "thread_count".into(),
                SqlValue::Integer(item.ids.len() as i64),
            ),
        ]],
        conflict_key: Some("id"),
        exclude_from_update: vec![],
        update_guard: None,
        version_column: Some("topic_inventory_version"),
    }))
}

/// 旧协议数量只可更新尚未建立全集版本的父群；版本0不能覆盖新版权威。
pub(crate) fn legacy_count_op(parent: ChannelId, value: &serde_json::Value) -> Option<StorageOp> {
    let count = value.get("threadCount")?.as_i64().filter(|v| *v >= 0)?;
    Some(StorageOp::BatchUpsert(UpsertSpec {
        table: "channel",
        conflict_key: Some("id"),
        exclude_from_update: vec![],
        update_guard: None,
        version_column: Some("topic_inventory_version"),
        rows: vec![vec![
            ("id".into(), SqlValue::Text(parent.as_str().into())),
            ("topic_inventory_version".into(), SqlValue::Integer(0)),
            ("thread_count".into(), SqlValue::Integer(count)),
        ]],
    }))
}
