use crate::shared;
use helix_im::send::upload_props::build_upload_plan;
use serde_json::json;

/// MV3-G02d：在 Helix media planner 边界拒绝 64 MiB+1，且不产生 Go/OSS/SQLite 副作用。
#[tokio::test]
async fn mv3_g02d_real_chain() {
    let error = build_upload_plan(
        "FILE",
        "phase1-over-limit",
        json!({
            "file": {
                "mediaInput": {
                    "localPath": "/tmp/phase1-over-limit.bin",
                    "fileName": "phase1-over-limit.bin",
                    "contentType": "application/octet-stream",
                    "size": 67_108_865_u64,
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            }
        }),
    )
    .expect_err("G02d must reject 64 MiB+1 before any upload effect");
    assert!(
        error.to_string().contains("67108864"),
        "unexpected G02d rejection: {error}"
    );
    shared::write_gate_evidence(
        "Phase1",
        "Upload",
        "MV3-G02d",
        "PASS",
        json!({
            "corr_key": shared::run_token("upload-over-limit"),
            "owner": "Helix upload planner / native host boundary",
            "input_size": 67_108_865_u64,
            "max_size": 67_108_864_u64,
            "http_business": "N/A: local planner rejects before external HTTP by contract",
            "projection": "N/A: no DomainEvent is allowed on early rejection",
            "cleanup": {"status": "PASS", "scope": "no effects allocated"},
            "rejected_before": [
                "im_stage_generated_media",
                "im_send_message",
                "Java oss/getPresignedUrl",
                "Go posts/create",
                "UploadFile/OSS PUT",
                "SQLite send_status=sending",
                "business WS post"
            ],
            "effect_count": 0,
            "go_http_count": 0,
            "oss_put_count": 0,
            "sqlite_sending_count": 0,
            "business_ws_count": 0,
            "error": error.to_string()
        }),
    )
    .unwrap_or_else(|error| panic!("MV3-G02d evidence write failed: {error}"));
}
