use crate::shared;

/// MV3-G13e：冷启动/重连生命周期是本地宿主状态，不是 Go mutation Gate。
#[tokio::test]
async fn mv3_g13e_real_chain() {
    shared::record_not_applicable(
        "Phase2",
        "Sync",
        "MV3-G13e",
        "G13e 只验证 Angular/native 生命周期与恢复调度；没有独立 Go mutation，真实 Go 链不能替代本地状态机断言。",
    )
    .unwrap();
}
