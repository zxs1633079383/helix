use crate::shared;

/// CHAIN-CAT-G02：分类选项在生产能力落地前不得产生伪 HTTP 或伪成功 Effect。
#[test]
fn chain_cat_g02_real_chain() {
    shared::run_unimplemented_chain_gate("CHAIN-CAT-G02")
        .unwrap_or_else(|error| panic!("CHAIN-CAT-G02 FAIL: {error}"));
}
