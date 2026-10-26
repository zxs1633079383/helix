use crate::shared;

/// CHAIN-CAT-G01：分类模式在生产能力落地前必须从公开 Command seam fail-closed。
#[test]
fn chain_cat_g01_real_chain() {
    shared::run_unimplemented_chain_gate("CHAIN-CAT-G01")
        .unwrap_or_else(|error| panic!("CHAIN-CAT-G01 FAIL: {error}"));
}
