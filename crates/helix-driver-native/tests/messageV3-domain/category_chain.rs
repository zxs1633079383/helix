//! Native 分类接龙真实链：Tick → Go HTTP/WS → SQLite → EventSink。

use crate::shared;
use futures_util::FutureExt;
use helix_core::effect::ScanSpec;
use helix_core::ports::Storage;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;

use tokio::time::Duration;

const SESSION_FILE: &str = "/tmp/category-isolated-service-a03f8b2c062a/private-session.json";

#[derive(Debug, Deserialize)]
struct PrivateSession {
    #[serde(rename = "companyId")]
    company_id: String,
    #[serde(rename = "channelId")]
    channel_id: String,
    #[serde(rename = "httpBase")]
    http_base: String,
    #[serde(rename = "wsURL")]
    ws_url: String,
    owner: FixtureIdentity,
    member: FixtureIdentity,
    outsider: FixtureIdentity,
}

#[derive(Debug, Deserialize)]
struct FixtureIdentity {
    #[serde(rename = "userId")]
    user_id: String,
    #[serde(rename = "cookieId")]
    cookie_id: String,
    #[allow(dead_code)]
    headers: BTreeMap<String, String>,
}

/// 读取本轮隔离服务身份；缺失前置直接失败。
fn load_private_session() -> Result<PrivateSession, String> {
    let content = std::fs::read_to_string(
        std::env::var("CATEGORY_NATIVE_SESSION_FILE").unwrap_or_else(|_| SESSION_FILE.to_owned()),
    )
    .map_err(|error| format!("BLOCKED_PRECONDITION: read isolated private session: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("BLOCKED_PRECONDITION: parse isolated private session: {error}"))
}

/// 把私有 fixture 转为现有 Native 登录上下文。
fn persona(identity: &FixtureIdentity, company_id: &str) -> shared::Persona {
    shared::Persona {
        user_id: identity.user_id.clone(),
        cookie_id: identity.cookie_id.clone(),
        company_id: company_id.to_owned(),
        user_name: String::new(),
        org_name: String::new(),
        dept_name: String::new(),
        companies: vec![company_id.to_owned()],
    }
}

/// 保持 actor、observer 与 outsider 身份边界。
fn context(session: &PrivateSession) -> shared::AuthContext {
    shared::AuthContext {
        actor: persona(&session.owner, &session.company_id),
        observer: persona(&session.member, &session.company_id),
        third: persona(&session.outsider, &session.company_id),
    }
}

/// 解析 Native EventSink 的规范事件。
fn event_value(event: &helix_core::effect::DomainEventBytes) -> Value {
    serde_json::from_slice(event.0.as_ref()).expect("DomainEvent must be JSON")
}

/// 仅接受匹配 mutation 的已持久化成功终态。
async fn wait_operation(
    session: &mut shared::RealHelixSession,
    mutation: &str,
) -> Result<Value, String> {
    let event = session
        .wait_event(
            "im:category_chain:operation",
            |envelope| {
                envelope
                    .pointer("/data/clientMutationId")
                    .and_then(Value::as_str)
                    == Some(mutation)
                    && envelope.pointer("/data/state").and_then(Value::as_str) == Some("CONFIRMED")
            },
            Duration::from_secs(30),
        )
        .await?;
    Ok(event_value(&event))
}

/// 读取本轮 SQLite 事实，不查询远端业务数据。
async fn scan_rows(
    storage: &helix_driver_native::NativeStorage,
    table: &'static str,
) -> Result<Vec<helix_core::effect::Row>, String> {
    storage
        .scan(ScanSpec {
            table,
            limit: Some(256),
            filter: None,
            order_by: &[],
        })
        .await
        .map_err(|error| format!("scan {table}: {error}"))
}

/// 收集对账的全部事件直到读终态，拒绝任何公共消息回放并核对私有终态次数。
async fn history_result(
    session: &mut shared::RealHelixSession,
    expected_operations: usize,
) -> Value {
    let mut operations = Vec::new();
    let mut read_seen = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), session.events.recv())
            .await
            .unwrap()
            .unwrap();
        let value = event_value(&event);
        let name = value["event"].as_str().unwrap_or("");
        assert!(
            !["im:post:sent", "im:post:received", "im:post:updated"].contains(&name),
            "historical receipt emitted public post"
        );
        if name == "im:category_chain:operation" {
            operations.push(value.clone());
        }
        if name == "im:read:result" && value["data"]["req_id"] == "native-history" {
            assert_eq!(value["data"]["body"]["status"], "SUCCESS");
            read_seen = true;
        }
        if read_seen && operations.len() >= expected_operations {
            assert_eq!(operations.len(), expected_operations);
            return operations.into_iter().next().unwrap_or(Value::Null);
        }
    }
}

/// 覆盖 capabilities/create(max_categories=2)/publish 的公开 Tick 生命周期。
#[tokio::test]
async fn category_chain_native() {
    let fixture = load_private_session().unwrap_or_else(|error| panic!("{error}"));
    let context = context(&fixture);

    let mut owner = shared::start_real_helix_session_as_at(
        &context,
        &context.actor,
        &[fixture.channel_id.as_str()],
        &fixture.http_base,
        &fixture.ws_url,
    )
    .await
    .unwrap_or_else(|error| panic!("category native session start failed: {error}"));

    let mut member = shared::start_real_helix_session_as_at(
        &context,
        &context.observer,
        &[fixture.channel_id.as_str()],
        &fixture.http_base,
        &fixture.ws_url,
    )
    .await
    .unwrap();
    let mut recovery = shared::start_real_helix_session_as_at(
        &context,
        &context.observer,
        &[fixture.channel_id.as_str()],
        &fixture.http_base,
        &fixture.ws_url,
    )
    .await
    .unwrap();
    let mut evidence_path = None;
    let result = AssertUnwindSafe(async {
    owner
        .command(
            "category_chain_capabilities",
            json!({"req_id": "category-native-capabilities"}),
        )
        .await
        .unwrap();
    let capabilities = owner
        .wait_event(
            "im:read:result",
            |event| event.pointer("/data/req_id").and_then(Value::as_str)
                == Some("category-native-capabilities"),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
    let capabilities = event_value(&capabilities);
    assert_eq!(capabilities["data"]["body"]["status"], "SUCCESS");
    assert_eq!(
        capabilities["data"]["body"]["data"]["categoryChain"]["schemaVersion"],
        1
    );
    assert_eq!(
        capabilities["data"]["body"]["data"]["categoryChain"]["supported"],
        true
    );

    let create_mutation = shared::run_token("category-create");
    owner
        .command(
            "category_chain_create_draft",
            json!({
                "channel_id": fixture.channel_id,
                "title": "Native 分类验收",
                "description": "Native 分类真实链",
                "example": "测试",
                "deadline_at": 4_000_000_000_000_i64,
                "audience": {"mode": "CHANNEL_SNAPSHOT"},
                "categories": [
                    {"clientId": "indoor", "name": "室内"},
                    {"clientId": "outdoor", "name": "户外"}
                ],
                "max_categories": 2,
                "client_mutation_id": create_mutation
            }),
        )
        .await
        .unwrap();
    let created = wait_operation(&mut owner, &create_mutation)
        .await
        .unwrap_or_else(|error| panic!("category create failed: {error}"));
    assert_eq!(created["data"]["state"], "CONFIRMED");
    assert_eq!(created["data"]["draft"]["chain"]["maxCategories"], 2);
    let chain_id = created["data"]["chainId"]
        .as_str()
        .or_else(|| created.pointer("/data/draft/chain/id").and_then(Value::as_str))
        .expect("create operation must return chainId")
        .to_owned();
    let definition_revision = created["data"]["revision"].as_str().unwrap_or("1").to_owned();

    let publish_mutation = shared::run_token("category-publish");
    owner
        .command(
            "category_chain_publish",
            json!({
                "channel_id": fixture.channel_id,
                "chain_id": chain_id,
                "expected_definition_revision": definition_revision,
                "client_mutation_id": publish_mutation
            }),
        )
        .await
        .unwrap();
    let published = wait_operation(&mut owner, &publish_mutation)
        .await
        .unwrap_or_else(|error| panic!("category publish failed: {error}"));
    assert_eq!(published["data"]["state"], "CONFIRMED");
    assert_eq!(published["data"]["card"]["maxCategories"], 2);
    assert_eq!(published["data"]["post"]["type"], "CATEGORY_CHAIN");
    assert_eq!(
        published["data"]["post"]["props"]["categoryChain"],
        published["data"]["card"]
    );

    let temporary_id = published["data"]["post"]["temporaryId"]
        .as_str()
        .expect("publish post must carry temporaryId")
        .to_owned();
    let owner_message = shared::scan_message_row(&owner.storage, &temporary_id)
        .await
        .unwrap_or_else(|error| panic!("owner message readback failed: {error}"));
    assert_eq!(shared::row_text(&owner_message, "type").as_deref(), Some("CATEGORY_CHAIN"));
    let owner_heads = scan_rows(&owner.storage, "category_chain_head").await.unwrap();
    let owner_facts = scan_rows(&owner.storage, "category_chain_fact").await.unwrap();
    assert!(owner_heads.iter().any(|row| {
        shared::row_text(row, "scope")
            .is_some_and(|scope| scope.contains(&fixture.channel_id) && scope.contains(&chain_id))
    }));
    assert!(owner_facts.iter().any(|row| {
        shared::row_text(row, "snapshot_key")
            .is_some_and(|scope| scope.contains(&fixture.channel_id) && scope.contains(&chain_id))
    }));

    let observer_event = member.wait_event("im:post:received", |v| v.pointer("/data/temporaryId").and_then(Value::as_str)==Some(&temporary_id), Duration::from_secs(20)).await.unwrap();
    assert_eq!(event_value(&observer_event)["data"]["type"], "CATEGORY_CHAIN");
    assert_eq!(shared::row_text(&shared::scan_message_row(&member.storage, &temporary_id).await.unwrap(), "type").as_deref(), Some("CATEGORY_CHAIN"));
    let category_id = published["data"]["card"]["categories"][0]["categoryId"].as_str().expect("category id").to_owned();
    let mutation_a = shared::run_token("category-participate-a");
    member.command("category_chain_set_participation", json!({"channel_id":fixture.channel_id,"chain_id":chain_id,"expected_participation_revision":"0","selections":[{"categoryId":category_id,"content":"A"}],"client_mutation_id":mutation_a})).await.unwrap();
    let receipt_a=wait_operation(&mut member,&mutation_a).await.unwrap();
    let revision_a=receipt_a["data"]["card"]["revision"].as_str().unwrap().to_owned();
    let participation_revision=receipt_a["data"]["myParticipation"]["participationRevision"].as_str().unwrap().to_owned();
    let mutation_b=shared::run_token("category-participate-b");
    member.command("category_chain_set_participation",json!({"channel_id":fixture.channel_id,"chain_id":chain_id,"expected_participation_revision":participation_revision,"selections":[],"client_mutation_id":mutation_b})).await.unwrap();
    let receipt_b=wait_operation(&mut member,&mutation_b).await.unwrap();
    let revision_b=receipt_b["data"]["card"]["revision"].as_str().unwrap().to_owned();
    assert!(revision_b.parse::<u64>().unwrap()>revision_a.parse::<u64>().unwrap());
    recovery.command("category_chain_get",json!({"channel_id":fixture.channel_id,"chain_id":chain_id,"req_id":"native-new-head"})).await.unwrap();
    let fresh=recovery.wait_event("im:read:result",|v|v["data"]["req_id"]=="native-new-head",Duration::from_secs(20)).await.unwrap();
    assert_eq!(event_value(&fresh)["data"]["body"]["status"],"SUCCESS");
    let before=shared::scan_message_row(&recovery.storage,&temporary_id).await.unwrap();
    while recovery.events.try_recv().is_ok() {}
    let reconcile=json!({"channel_id":fixture.channel_id,"chain_id":chain_id,"client_mutation_id":mutation_a,"req_id":"native-history"});
    recovery.command("category_chain_reconcile",reconcile.clone()).await.unwrap();
    let historical=history_result(&mut recovery,1).await;
    assert_eq!(historical["data"]["card"]["revision"],revision_a);
    assert_eq!(format!("{:?}",shared::scan_message_row(&recovery.storage,&temporary_id).await.unwrap()),format!("{:?}",before));
    recovery.command("category_chain_reconcile",reconcile).await.unwrap();
    history_result(&mut recovery,0).await;
    // Only this session's disposable SQLite is damaged; the Go business service remains healthy.
    recovery.storage.execute_raw("DROP TABLE category_chain_fact").await.unwrap();
    recovery.command("category_chain_get",json!({"channel_id":fixture.channel_id,"chain_id":chain_id,"req_id":"native-persist-failure"})).await.unwrap();
    let failed=recovery.wait_event("im:read:result",|v|v["data"]["req_id"]=="native-persist-failure",Duration::from_secs(20)).await.unwrap();
    assert!(event_value(&failed)["data"]["error"].as_str().is_some());
    assert_eq!(format!("{:?}",shared::scan_message_row(&recovery.storage,&temporary_id).await.unwrap()),format!("{:?}",before));
    let evidence=json!({"status":"VALIDATED_CLEANUP_PENDING","channelId":fixture.channel_id,"chainId":chain_id,"revisionA":revision_a,"revisionB":revision_b,"checks":["capabilities","create","publish","observerNative","participation","historicalReceipt","duplicate","sqliteFailure"]});
    let dir=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/messagev3-phase1-4-real-chain").join(shared::run_token("category-native"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("category.json"),serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("category native evidence: {}",dir.display());
    evidence_path=Some(dir.join("category.json"));
    }).catch_unwind().await;
    let owner_close = owner.close().await;
    let member_close = member.close().await;
    let recovery_close = recovery.close().await;
    owner_close.unwrap();
    member_close.unwrap();
    recovery_close.unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    let path = evidence_path.expect("validated evidence");
    let mut evidence: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    evidence["status"] = json!("PASS");
    evidence["cleanup"] = json!({"nativeSessionsClosed":3,"ownedSqliteRemoved":true,"remoteSchemaOwner":"isolated-service"});
    std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
}
