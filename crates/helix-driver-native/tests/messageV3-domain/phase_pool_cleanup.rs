//! MessageV3 Phase 批次共享资源的最终清理入口。

use crate::shared;

/// Phase1 批次末统一关闭共享账号 WS、SQLite 与五个远端频道。
#[tokio::test]
async fn phase1_z_pool_cleanup_real_chain() {
    shared::shutdown_messagev3_phase_pool(1)
        .await
        .unwrap_or_else(|error| panic!("Phase1 pooled cleanup failed: {error}"));
}

/// Phase2 批次末统一关闭共享账号 WS、SQLite 与五个远端频道。
#[tokio::test]
async fn phase2_z_pool_cleanup_real_chain() {
    shared::shutdown_messagev3_phase_pool(2)
        .await
        .unwrap_or_else(|error| panic!("Phase2 pooled cleanup failed: {error}"));
}

/// Phase3 批次末统一关闭共享账号 WS、SQLite 与五个远端频道。
#[tokio::test]
async fn phase3_z_pool_cleanup_real_chain() {
    shared::shutdown_messagev3_phase_pool(3)
        .await
        .unwrap_or_else(|error| panic!("Phase3 pooled cleanup failed: {error}"));
}

/// Phase4 批次末统一关闭共享账号 WS、SQLite 与五个远端频道。
#[tokio::test]
async fn phase4_z_pool_cleanup_real_chain() {
    shared::shutdown_messagev3_phase_pool(4)
        .await
        .unwrap_or_else(|error| panic!("Phase4 pooled cleanup failed: {error}"));
}

/// Phase5 批次末统一关闭共享账号 WS、SQLite 与五个远端频道。
#[tokio::test]
async fn phase5_z_pool_cleanup_real_chain() {
    shared::shutdown_messagev3_phase_pool(5)
        .await
        .unwrap_or_else(|error| panic!("Phase5 pooled cleanup failed: {error}"));
}
