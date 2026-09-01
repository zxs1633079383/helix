use crate::shared;

/// CHAIN-ACC-G01：跨 TEXT/Category 验收在 Category 缺失时必须如实阻断。
#[test]
fn chain_acc_g01_real_chain() {
    shared::run_unimplemented_chain_gate("CHAIN-ACC-G01")
        .unwrap_or_else(|error| panic!("CHAIN-ACC-G01 FAIL: {error}"));
}
