//! Timeline V3 共用 pageSize 与定位配额策略合同。

use helix_im::projector::timeline::{
    allocate_locate_window, TimelinePageSize, DEFAULT_TIMELINE_PAGE_SIZE, MAX_TIMELINE_PAGE_SIZE,
};
use helix_im::query::{message_scan_spec, parse_message_query};
use serde_json::json;

const CHANNEL_ID: &str = "chpolicy000000000000000001";

/// 序列化 latest 查询并返回解析后的 pageSize。
fn parsed_page_size(page_size: Option<u64>) -> Result<u32, helix_im::ImError> {
    let mut payload = json!({ "channel_id": CHANNEL_ID });
    if let Some(page_size) = page_size {
        payload["pageSize"] = json!(page_size);
    }
    parse_message_query(&serde_json::to_vec(&payload).expect("policy payload serializes"))
        .map(|request| request.limit)
}

/// 证明缺省值为 20，边界 1/60 均保留且 storage scan 只多取一条 lookahead。
#[test]
fn latest_page_size_defaults_and_bounds_are_exact() {
    assert_eq!(
        parsed_page_size(None).expect("missing pageSize defaults"),
        DEFAULT_TIMELINE_PAGE_SIZE
    );
    assert_eq!(parsed_page_size(Some(1)).expect("one is valid"), 1);
    assert_eq!(
        parsed_page_size(Some(60)).expect("sixty is valid"),
        MAX_TIMELINE_PAGE_SIZE
    );

    let request = parse_message_query(
        serde_json::to_vec(&json!({
            "channel_id": CHANNEL_ID,
            "pageSize": 60
        }))
        .expect("scan payload serializes")
        .as_slice(),
    )
    .expect("pageSize 60 parses");
    assert_eq!(message_scan_spec(&request).limit, Some(61));
}

/// 证明 0/61/非整数在产生任何 Effect 之前 fail closed。
#[test]
fn invalid_page_size_is_rejected_before_io() {
    for invalid in [json!(0), json!(61), json!(1.5), json!("20")] {
        let payload = serde_json::to_vec(&json!({
            "channel_id": CHANNEL_ID,
            "pageSize": invalid
        }))
        .expect("invalid payload serializes");
        assert!(parse_message_query(&payload).is_err());
    }
}

/// 证明旧 limit 别名也遵守同一 1..=60 策略，不能继续 clamp 到合法值。
#[test]
fn legacy_limit_alias_uses_the_same_fail_closed_policy() {
    for invalid in [0_u64, 61, 500] {
        let payload = serde_json::to_vec(&json!({
            "channel_id": CHANNEL_ID,
            "limit": invalid
        }))
        .expect("legacy payload serializes");
        assert!(parse_message_query(&payload).is_err());
    }
}

/// 证明双侧充足时目标位于 floor(N/2)，并且目标计入总页长。
#[test]
fn locate_allocation_centers_target_when_both_sides_are_sufficient() {
    for page_size in [20_u32, 60] {
        let page_size = TimelinePageSize::new(page_size).expect("bounded page size");
        let allocation = allocate_locate_window(100, 100, page_size);
        assert_eq!(allocation.before, page_size.get() as usize / 2);
        assert_eq!(
            allocation.after,
            page_size.get() as usize - 1 - allocation.before
        );
        assert_eq!(allocation.target_index(), allocation.before);
        assert_eq!(allocation.total(), page_size.get() as usize);
    }
}

/// 证明任一侧不足时把未使用额度确定性补给另一侧。
#[test]
fn locate_allocation_refills_the_longer_side() {
    let page_size = TimelinePageSize::new(20).expect("bounded page size");
    let older_short = allocate_locate_window(3, 100, page_size);
    assert_eq!((older_short.before, older_short.after), (3, 16));
    assert_eq!(older_short.total(), 20);

    let newer_short = allocate_locate_window(100, 2, page_size);
    assert_eq!((newer_short.before, newer_short.after), (17, 2));
    assert_eq!(newer_short.total(), 20);
}
