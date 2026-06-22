//! F5 — Golden Tick 剧本回放（确定性 + 回归捕捉）
//!
//! ## 目的（ROADMAP §一 M3 F5）
//!
//! 用 `ExecutionShell`（真引擎 API，驱动实际使用的接口）驱动剧本，
//! 收集 `Effect::Emit` 产出的 `DomainEventBytes` 序列，与已提交的 golden fixture
//! byte-identical 断言，作为跨端 parity 的确定性底座。
//!
//! ## 剧本（手写，im_trace 自动导出管线待 ROADMAP P0 golden 工具链）
//!
//! Tick 1: `im_send_message`（channel 1，tmp_id="f5_tmp_1"，text="hello-f5"）
//!   → 产 `Emit(im:post:sending)`
//! Tick 2: 真 Go `post` 帧（tests/fixtures/real_go_frames/post.json，event_seq=40114，
//!          channel=rmibkwkjwidhdx5xy8pyse6a3r，cursor 预种 40113 → 立即 apply）
//!   → 产 `Emit(im:post:received)`
//!
//! ## HX-C011 可证伪性
//!
//! golden fixture NDJSON 行数必须 == Emit 条数，且内容 byte-identical。
//! 若改 acl/to_effect.rs 任意一个 emit_* 的字段顺序或名称 → fixture mismatch FAIL。
//! commit body 附「临时改一处 emit → 测 FAIL → 还原 → PASS」破坏证明（见提交记录）。
//!
//! ## 确定性验证
//!
//! 整段剧本跑两次，两次收集到的 DomainEvent 字节序列 byte-identical 断言。

use helix_core::effect::DomainEventBytes;
use helix_core::tick::{AppCommand, InboundBytes};
use helix_core::{Effect, ExecutionShell, Tick};
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;
use helix_im::ImModule;

// ─── 固定时间戳（注入 step，保证确定性）────────────────────────────────────────
const FIXED_NOW_MS: u64 = 1_700_000_000_000;

// ─── 真 Go post 帧（复用 golden_replay_test.rs 的 fixture）──────────────────────
const POST_JSON: &str = include_str!("fixtures/real_go_frames/post.json");

/// 剥掉 `//` 头注释行，返回纯 JSON 字节。
fn fixture_body(raw: &str) -> Vec<u8> {
    raw.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

// ─── 引擎组装（每次剧本独立创建，保证无状态污染）──────────────────────────────

fn build_shell() -> ExecutionShell {
    let config = ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: String::new(),
        company_id: String::new(),
    };
    let mut module = ImModule::new(config);

    // Tick 1 用的 channel（数值 id=1 → 26 字符 "00000000000000000000000001"）
    module.register_channel(ChannelId::from_test_u64(1), 0);

    // Tick 2 用的真帧 channel（post.json channelId=rmibkwkjwidhdx5xy8pyse6a3r，event_seq=40114）
    // cursor 预种 40113 → event_seq(40114)==cursor+1 → 立即 apply
    let post_channel =
        ChannelId::from_str("rmibkwkjwidhdx5xy8pyse6a3r").expect("真帧 channelId 合法 26 字符");
    module.register_channel(post_channel, 40113);

    let mut shell = ExecutionShell::new();
    shell.register(module);
    shell
}

// ─── 剧本执行（收集所有 Emit 的 DomainEventBytes）────────────────────────────────

fn run_script(shell: &mut ExecutionShell) -> Vec<DomainEventBytes> {
    let mut emits: Vec<DomainEventBytes> = Vec::new();

    // start()：收集 on_start Effect（通常含 Persist scan + ScheduleTimer ping，不含 Emit）
    let start_effects = shell.start().expect("start() should not fail");
    collect_emits(start_effects, &mut emits);

    // Tick 1: im_send_message → Emit(im:post:sending)
    let payload = serde_json::json!({
        "channel_id": "00000000000000000000000001",
        "temporary_id": "f5_tmp_1",
        "text": "hello-f5"
    });
    let cmd = Tick::Command(AppCommand::new(
        "im_send_message",
        serde_json::to_vec(&payload).expect("static JSON"),
    ));
    let effects = shell
        .step(cmd, FIXED_NOW_MS)
        .expect("step cmd should not fail");
    collect_emits(effects, &mut emits);

    // Tick 2: 真 Go post 帧 → Emit(im:post:received)
    let post_tick = Tick::Inbound(InboundBytes(fixture_body(POST_JSON).into()));
    let effects = shell
        .step(post_tick, FIXED_NOW_MS)
        .expect("step inbound should not fail");
    collect_emits(effects, &mut emits);

    emits
}

fn collect_emits(effects: &[Effect], out: &mut Vec<DomainEventBytes>) {
    for e in effects {
        if let Effect::Emit { event } = e {
            out.push(event.clone());
        }
    }
}

// ─── golden fixture I/O ──────────────────────────────────────────────────────

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/f5/f5_golden_events.ndjson"
);

/// 首次生成 golden fixture：跑剧本 + 把 Emit 序列写为 NDJSON（每行一条）。
///
/// 正常 CI 跑 `f5_golden_replay_matches_fixture`（读 fixture）；
/// 只有 **fixture 尚不存在或需要刻意更新** 时才 `cargo test -- --ignored` 跑本测试。
#[test]
#[ignore = "首次生成 fixture 时手动跑：cargo test -p helix-im -- f5_write_golden_fixture --ignored"]
fn f5_write_golden_fixture() {
    let mut shell = build_shell();
    let emits = run_script(&mut shell);
    assert!(!emits.is_empty(), "剧本必须产出至少一条 Emit");
    let lines: Vec<String> = emits
        .iter()
        .map(|e| String::from_utf8(e.0.to_vec()).expect("DomainEvent 必须是合法 UTF-8 JSON"))
        .collect();
    std::fs::create_dir_all(
        std::path::Path::new(GOLDEN_PATH)
            .parent()
            .expect("GOLDEN_PATH 有父目录"),
    )
    .expect("创建 fixture 目录");
    std::fs::write(GOLDEN_PATH, lines.join("\n") + "\n").expect("写 fixture 文件");
    // 写完后把内容 assert 回打，方便 --nocapture 时人眼核查
    assert!(
        !lines.is_empty(),
        "写入 {} 条 golden events → {GOLDEN_PATH}",
        lines.len()
    );
}

/// golden fixture 读取：每行一条 DomainEvent JSON bytes。空行忽略。
fn read_golden() -> Vec<bytes::Bytes> {
    std::fs::read_to_string(GOLDEN_PATH)
        .expect("golden fixture 必须存在；若首次运行先执行 write_golden_fixture 生成")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| bytes::Bytes::from(l.to_string()))
        .collect()
}

// ─── 测试 ────────────────────────────────────────────────────────────────────

/// golden 回放：ExecutionShell 剧本产出的 Emit 序列与 committed fixture byte-identical。
///
/// HX-C011 可证伪：若改任何 emit_* 字段 → fixture mismatch FAIL。
#[test]
fn f5_golden_replay_matches_fixture() {
    let mut shell = build_shell();
    let emits = run_script(&mut shell);

    // golden 行数必须 == Emit 条数（防 fixture 静默少于实际）
    let golden = read_golden();
    assert_eq!(
        emits.len(),
        golden.len(),
        "golden fixture 行数({}) 必须 == 剧本 Emit 条数({})\n剧本 emits = {:?}",
        golden.len(),
        emits.len(),
        emits
            .iter()
            .map(|e| String::from_utf8_lossy(&e.0).into_owned())
            .collect::<Vec<_>>(),
    );

    // byte-identical 逐行断言
    for (i, (actual, expected)) in emits.iter().zip(golden.iter()).enumerate() {
        assert_eq!(
            actual.0,
            *expected,
            "Emit[{}] byte mismatch:\n  actual  = {}\n  golden  = {}",
            i,
            String::from_utf8_lossy(&actual.0),
            String::from_utf8_lossy(expected),
        );
    }
}

/// 确定性：同一剧本跑两次，两次 Emit 序列 byte-identical。
///
/// 防 HashMap 随机 SipHash 序（HX-C010）或时间注入不一致引入不确定性。
#[test]
fn f5_golden_replay_deterministic_twice() {
    let emits_1 = {
        let mut shell = build_shell();
        run_script(&mut shell)
    };
    let emits_2 = {
        let mut shell = build_shell();
        run_script(&mut shell)
    };

    assert_eq!(
        emits_1.len(),
        emits_2.len(),
        "两次剧本 Emit 条数不同（{}!={})——非确定性",
        emits_1.len(),
        emits_2.len()
    );
    for (i, (a, b)) in emits_1.iter().zip(emits_2.iter()).enumerate() {
        assert_eq!(
            a.0,
            b.0,
            "Emit[{i}] 两次结果不同（非确定性）:\n  run1 = {}\n  run2 = {}",
            String::from_utf8_lossy(&a.0),
            String::from_utf8_lossy(&b.0),
        );
    }
}
