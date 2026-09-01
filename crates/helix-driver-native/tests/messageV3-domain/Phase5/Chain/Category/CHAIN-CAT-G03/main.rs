use crate::shared;

/// CHAIN-CAT-G03：分类统计不得在缺少 Category producer 时被本地 fixture 冒充完成。
#[test]
fn chain_cat_g03_real_chain() {
    shared::run_unimplemented_chain_gate("CHAIN-CAT-G03")
        .unwrap_or_else(|error| panic!("CHAIN-CAT-G03 FAIL: {error}"));
}
