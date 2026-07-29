//! Sync Gate 的持久释放与 artifact 公共回放。

use helix_core::effect::{Row, SqlValue, StorageOp, UpsertSpec};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_im::event::MessageV3Event;
use serde_json::{json, Value};

#[path = "../support/artifact.rs"]
pub mod artifact;
#[path = "../support/scenario.rs"]
mod scenario;

pub type TestResult<T> = Result<T, String>;

/// 保存真实 PersistOk 前后观察结果。
pub struct SyncTrace {
    pub events: Vec<Value>,
    pub ticks: Vec<Value>,
}

/// 用 Gate 对应事件通过生产 commit coordinator 验证持久屏障。
pub fn run(event: MessageV3Event) -> TestResult<SyncTrace> {
    let mut replay = scenario::Scenario::new(helix_im::ImConfig {
        auth_user_id: "sync-viewer".to_string(),
        company_id: "sync-company".to_string(),
        ..helix_im::ImConfig::default()
    });
    let row: Row = vec![
        ("id".to_string(), SqlValue::Text("sync-state".to_string())),
        ("value".to_string(), SqlValue::Text("committed".to_string())),
    ];
    let corr = replay.queue_commit(
        vec![StorageOp::BatchUpsert(UpsertSpec::new(
            "meta",
            vec![row],
            Some("id"),
        ))],
        vec![event],
        1_700_001_700_000,
    );
    assert!(replay.emitted_events().is_empty());
    replay
        .reply(
            corr,
            PortOutcome::Ok(ReplyBytes::default()),
            1_700_001_700_001,
        )
        .map_err(|error| error.to_string())?;
    Ok(SyncTrace {
        events: replay.emitted_events(),
        ticks: replay.ticks().to_vec(),
    })
}

/// 写出单 viewer sync Gate artifact。
pub fn write_artifact(gate_id: &str, producer: &str, trace: SyncTrace) -> TestResult<()> {
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let storage = json!([{"table":"sync_cursor","columns":["channel_id","next_seq","updated_at"]}]);
    let bundle = artifact::gate_bundle(
        gate_id,
        producer,
        trace.ticks,
        trace.events,
        Vec::new(),
        storage.clone(),
        storage.clone(),
        storage,
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
