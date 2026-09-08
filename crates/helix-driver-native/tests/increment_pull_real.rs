//! run-owned 真 Go HTTP/WS + PostgreSQL → Native engine → SQLite → EventSink 恢复验收。
#[path = "messageV3-domain/shared/mod.rs"]
mod shared;

use helix_core::effect::ScanSpec;
use helix_core::ports::Storage;
use std::time::Duration;

/// 由 Go TestIncrementPageHTTPPressure 提供隔离夹具；缺少前置时不伪装通过。
#[tokio::test]
#[ignore = "requires run-owned Go increment fixture and explicit local endpoints"]
async fn increment_pull_real_go_sqlite_ready() {
    let dir = std::env::var("CSES_IM_INCREMENT_NATIVE_FIXTURE").expect("owned fixture directory");
    let fixture: serde_json::Value = serde_json::from_slice(
        &std::fs::read(format!("{dir}/ready.json")).expect("Go fixture readiness"),
    )
    .expect("fixture metadata");
    let expected = fixture["expected"]
        .as_u64()
        .expect("expected channel count") as usize;
    let actor = shared::Persona {
        user_id: "reader".into(),
        cookie_id: "reader".into(),
        company_id: "team-1".into(),
        user_name: "reader".into(),
        org_name: String::new(),
        dept_name: String::new(),
        companies: vec!["team-1".into()],
    };
    let context = shared::AuthContext {
        actor: actor.clone(),
        observer: actor.clone(),
        third: actor,
    };
    let result = async {
        let mut session = shared::start_real_helix_session(&context, &[]).await?;
        let assertion = async {
            session
                .wait_ws_action(
                    "hello",
                    |frame| frame["data"]["incrementPageVersion"] == 1,
                    Duration::from_secs(5),
                )
                .await?;
            session
                .wait_event("im:channel-sync-ready", |_| true, Duration::from_secs(60))
                .await?;
            let rows = session
                .storage
                .scan(ScanSpec {
                    table: "channel",
                    filter: None,
                    limit: None,
                    order_by: &[],
                })
                .await
                .map_err(|error| error.to_string())?;
            if rows.len() != expected {
                return Err(format!(
                    "ready before complete SQLite: {} != {expected}",
                    rows.len()
                ));
            }
            eprintln!(
                "PULL_REAL_GO_PASS negotiated=1 SQLite_channels={} terminal=im:channel-sync-ready",
                rows.len()
            );
            Ok::<(), String>(())
        }
        .await;
        let cleanup = session.close().await;
        assertion.and(cleanup)
    }
    .await;
    std::fs::write(format!("{dir}/done"), b"native finished").expect("release owned Go fixture");
    result.expect("real Go → Native → SQLite → EventSink recovery");
}
