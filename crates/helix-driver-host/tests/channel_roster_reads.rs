//! 公开 IM 查询 → 真实文件 SQLite → 完整名单/权限；任何网络补偿使测试失败。
use helix_core::effect::{ScopedScanSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const COMPANY: &str = "roster-company";
const VIEWER: &str = "viewer";
static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

/// 每例拥有真实 WAL 文件；包括失败退出时仅清理本例文件。
struct DatabaseFile(PathBuf);
impl Drop for DatabaseFile {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

async fn database() -> (DatabaseFile, HostStorage) {
    let file = DatabaseFile(std::env::temp_dir().join(format!(
        "helix-roster-{}-{}.db",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    assert!(!file.0.exists());
    let storage = HostStorage::open_sqlite_url(&format!("sqlite:{}?mode=rwc", file.0.display()))
        .await
        .unwrap();
    for sql in IM_SCHEMA.iter().chain(IM_SCHEMA_MIGRATIONS) {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{error}"
            );
        }
    }
    (file, storage)
}

fn module() -> ImModule {
    ImModule::new(ImConfig {
        auth_user_id: VIEWER.into(),
        company_id: COMPANY.into(),
        ..Default::default()
    })
}

fn channel(index: usize) -> String {
    format!("roster{index:020}")
}

/// 旧主表名单为空、角色错误；权威角色/全员只保存在channel_member。
async fn seed(storage: &HostStorage, groups: usize, count: usize) {
    for index in 0..groups {
        let id = channel(index);
        let text = |key: &str, value: String| (key.to_string(), SqlValue::Text(value));
        storage
            .batch_upsert(UpsertSpec::new(
                "channel",
                vec![vec![
                    text("id", id.clone()),
                    text("team_id", COMPANY.into()),
                    text("user_id", VIEWER.into()),
                    text("type", "P".into()),
                    text("display_name", format!("fixture-{index}")),
                    ("member_count".into(), SqlValue::Integer(count as i64)),
                    text("role", "MEMBER".into()),
                    text("mention_permission", "MANAGER".into()),
                    ("is_active".into(), SqlValue::Integer(1)),
                    text("members", "[]".into()),
                    text("admin_users", "[]".into()),
                ]],
                Some("id"),
            ))
            .await
            .unwrap();
        let rows = (0..count)
            .map(|n| {
                vec![
                    text("channel_id", id.clone()),
                    text(
                        "user_id",
                        if n == 1 {
                            VIEWER.into()
                        } else {
                            format!("member-{n}")
                        },
                    ),
                    text("team_id", COMPANY.into()),
                    text(
                        "role",
                        match n {
                            0 => "OWNER",
                            1 => "ADMIN",
                            _ => "MEMBER",
                        }
                        .into(),
                    ),
                    text("user_name", String::new()),
                    text("nick_name", String::new()),
                ]
            })
            .collect();
        storage
            .batch_upsert(UpsertSpec::new(
                "channel_member",
                rows,
                Some("channel_id,user_id"),
            ))
            .await
            .unwrap();
    }
}

/// 经真实端口回灌，失败不得伪造成空名单；网络、未完成循环立即失败。
async fn drain(storage: &HostStorage, module: &mut ImModule, mut sink: EffectSink) -> Vec<Value> {
    let mut events = Vec::new();
    for _ in 0..16 {
        if sink.is_empty() {
            return events;
        }
        let batch = std::mem::take(&mut sink);
        for effect in batch.as_slice() {
            let (corr, outcome) = match effect {
                Effect::Persist { corr, ops } => (*corr, execute_storage_ops(storage, ops).await),
                Effect::PersistAtomic { corr, ops } => {
                    (*corr, execute_storage_ops_atomic(storage, ops).await)
                }
                Effect::Emit { event } => {
                    events.push(serde_json::from_slice(event.0.as_ref()).unwrap());
                    continue;
                }
                Effect::ScheduleTimer { .. } | Effect::CancelTimer { .. } => continue,
                other => panic!("local roster query issued non-storage effect: {other:?}"),
            };
            assert!(
                matches!(outcome, PortOutcome::Ok(_)),
                "real HostStorage failed: {outcome:?}"
            );
            module
                .handle(&Tick::PortReply { corr, outcome }, 1, &mut sink)
                .unwrap();
        }
    }
    panic!("local query lifecycle did not finish");
}

async fn query(
    storage: &HostStorage,
    module: &mut ImModule,
    command: &'static str,
    args: Value,
) -> Value {
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new(command, serde_json::to_vec(&args).unwrap())),
            1,
            &mut sink,
        )
        .unwrap();
    let events = drain(storage, module, sink).await;
    let results: Vec<_> = events
        .iter()
        .filter(|event| {
            event["event"] == "im:read:result" && event["data"]["req_id"] == args["req_id"]
        })
        .collect();
    assert_eq!(results.len(), 1, "one matching result required");
    assert!(results[0]["data"].get("error").is_none(), "{}", results[0]);
    results[0]["data"]["body"].clone()
}

fn assert_roster(row: &Value, count: usize, can_mention: bool) {
    let mut ids = HashSet::new();
    for key in ["members", "adminUsers", "boss"] {
        for member in row[key].as_array().expect("complete role bucket") {
            assert!(ids.insert(member["userId"].as_str().unwrap()));
        }
    }
    assert!(ids.insert(row["owner"]["userId"].as_str().unwrap()));
    assert_eq!(ids.len(), count);
    assert_eq!(row["memberCount"], count);
    assert_eq!(
        row["adminUsers"].as_array().unwrap().len(),
        usize::from(can_mention)
    );
    assert_eq!(row["canMentionAll"], can_mention);
    assert_eq!(row["canManageMembers"], can_mention);
}

/// 最小10001扫描请求也必须由真实driver执行，并从权威成员修复管理员/权限。
#[tokio::test]
async fn single_5000_members_list_snapshot_offline_reopen_and_demotion() {
    let (file, storage) = database().await;
    seed(&storage, 1, 5000).await;
    let mut active = module();
    let list = query(
        &storage,
        &mut active,
        "im_query_dialog_list",
        json!({"req_id":"list"}),
    )
    .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_roster(&list["items"][0], 5000, true);
    drop(active);
    drop(storage);
    let reopened = HostStorage::open_sqlite_url(&format!("sqlite:{}?mode=rwc", file.0.display()))
        .await
        .unwrap();
    let mut offline = module();
    let snapshot = query(
        &reopened,
        &mut offline,
        "im_query_channel_view_snapshot",
        json!({"req_id":"snapshot", "channelId":channel(0)}),
    )
    .await;
    assert_roster(&snapshot["snapshot"], 5000, true);
    reopened
        .batch_upsert(UpsertSpec::new(
            "channel_member",
            vec![vec![
                ("channel_id".into(), SqlValue::Text(channel(0))),
                ("user_id".into(), SqlValue::Text(VIEWER.into())),
                ("role".into(), SqlValue::Text("MEMBER".into())),
            ]],
            Some("channel_id,user_id"),
        ))
        .await
        .unwrap();
    let downgraded = query(
        &reopened,
        &mut offline,
        "im_query_dialog_list",
        json!({"req_id":"demoted"}),
    )
    .await;
    assert_roster(&downgraded["items"][0], 5000, false);
}

/// 同一WAL上三个公开列表查询并发，均返回三个5000人群且无跨群成员混入。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_5000_member_groups_parallel_complete_reads() {
    let (_file, storage) = database().await;
    seed(&storage, 3, 5000).await;
    let read = || async {
        let mut active = module();
        let list = query(
            &storage,
            &mut active,
            "im_query_dialog_list",
            json!({"req_id":"parallel"}),
        )
        .await;
        assert_eq!(list["items"].as_array().unwrap().len(), 3);
        for row in list["items"].as_array().unwrap() {
            assert_roster(row, 5000, true);
        }
    };
    tokio::join!(read(), read(), read());
}

/// 真实绑定分块两侧：精确上限全部返回，超出一行失败而不是截断。
#[tokio::test]
async fn scoped_scan_10001_exact_and_overflow_sentinel_across_bind_chunks() {
    let (_file, storage) = database().await;
    storage.execute_raw("CREATE TABLE sentinel_rows(id INTEGER PRIMARY KEY, scope TEXT); WITH RECURSIVE seq(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM seq WHERE n<10000) INSERT INTO sentinel_rows SELECT n,printf('scope-%04d',n%1001) FROM seq").await.unwrap();
    let mut scopes: Vec<_> = (0..1001)
        .map(|n| SqlValue::Text(format!("scope-{n:04}")))
        .collect();
    scopes.push(scopes[0].clone());
    let scan = |limit| ScopedScanSpec {
        table: "sentinel_rows",
        scope_col: "scope",
        scope_values: scopes.clone(),
        filter: None,
        limit,
    };
    let rows = storage
        .scoped_scan(scan(10001))
        .await
        .expect("explicit 10001 limit must execute");
    assert_eq!(rows.len(), 10001);
    let ids: HashSet<_> = rows
        .iter()
        .map(
            |row| match &row.iter().find(|(k, _)| k == "id").unwrap().1 {
                SqlValue::Integer(id) => *id,
                other => panic!("unexpected ID {other:?}"),
            },
        )
        .collect();
    assert_eq!(
        ids.len(),
        10001,
        "duplicate scope values must not duplicate rows"
    );
    assert_eq!(*ids.iter().max().unwrap(), 10000);
    storage
        .execute_raw("INSERT INTO sentinel_rows VALUES(10001,'scope-1000')")
        .await
        .unwrap();
    let error = storage.scoped_scan(scan(10001)).await.unwrap_err();
    assert!(
        error.to_string().contains("exceeds explicit limit"),
        "{error}"
    );
    let recovered = storage.scoped_scan(scan(10002)).await.unwrap();
    assert_eq!(
        recovered.len(),
        10002,
        "failed scan must release its reader/transaction"
    );
}

/// 不可表示的LIMIT+1明确拒绝，空scope仍无SQL，零上限不默默返回部分数据。
#[tokio::test]
async fn scoped_scan_invalid_limit_empty_scope_and_zero_sentinel() {
    let (_file, storage) = database().await;
    let scan = |scope_values, limit| ScopedScanSpec {
        table: "channel",
        scope_col: "id",
        scope_values,
        filter: None,
        limit,
    };
    let error = storage
        .scoped_scan(scan(vec![SqlValue::Text(channel(0))], usize::MAX))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("sentinel"), "{error}");
    assert!(storage
        .scoped_scan(scan(vec![], usize::MAX))
        .await
        .unwrap()
        .is_empty());
    seed(&storage, 1, 2).await;
    let error = storage
        .scoped_scan(scan(vec![SqlValue::Text(channel(0))], 0))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("exceeds explicit limit"),
        "{error}"
    );
}

/// 同步页也从公开increment封口/PersistOk建立session，使用真实driver返回全量三群名单。
#[tokio::test]
async fn channel_sync_page_real_host_reads_three_complete_rosters() {
    let (_file, storage) = database().await;
    seed(&storage, 3, 5000).await;
    let mut active = module();
    let mut sink = EffectSink::new();
    let frame = json!({"action":"increment_channel","data":{
        "id":channel(0),"teamId":COMPANY,"userId":VIEWER,"type":"P","role":"ADMIN",
        "displayName":"fixture-0","memberCount":5000,"mentionPermission":"MANAGER",
        "lastEventSeq":0,"needSync":false,"unreadCount":0,"unReadPostId":""
    }});
    for frame in [frame, json!({"action":"increment_channel_end","data":{}})] {
        active
            .handle(
                &Tick::Inbound(InboundBytes(serde_json::to_vec(&frame).unwrap().into())),
                1,
                &mut sink,
            )
            .unwrap();
    }
    let events = drain(&storage, &mut active, sink).await;
    let ready = events
        .iter()
        .find(|event| event["event"] == "im:channel-sync-ready")
        .expect("PersistOk must establish session");
    let page=query(&storage,&mut active,"im_query_channel_sync_page",json!({"req_id":"page","channel_sync_session_id":ready["data"]["channelSyncSessionId"],"page_size":20})).await;
    assert_eq!(page["items"].as_array().unwrap().len(), 3);
    for row in page["items"].as_array().unwrap() {
        assert_roster(row, 5000, true);
    }
}
