use crate::shared;

/// MV3-G03a：保留 Angular item-forward 的真实前置缺口，不用错误 wire 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED: Angular item forward requires two authoritative local posts; current direct Go probe admitted but produced no target post WS"]
async fn mv3_g03a_real_chain() {
    shared::record_unverified(
        "Phase1",
        "Post",
        "MV3-G03a",
        "Angular item mode sends posts + channelIds from local authority rows; current target has no verified Helix fixture for that WS delivery",
    )
    .unwrap();
}
