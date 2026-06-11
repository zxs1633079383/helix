# REPORT — fix/e3-e4-helix-im

> worktree: `worktrees/fix-e34-im/`  branch: `fix/e3-e4-helix-im`
> 修复时间：2026-06-11  任务：E3 needs_continuation + E4 scan_corr + B1 gate_inflight

---

## §0 启动 pwd + branch 自检

```
pwd:    /Users/mac28/workspace/rustWorkspace/helix/worktrees/fix-e34-im  PASS
branch: fix/e3-e4-helix-im                                                PASS
```

---

## §1 Commit 链

| SHA | subject |
|---|---|
| `144b8fd` | fix(helix-im): E4 scan_corr PortReply 路由 + B1 gate_inflight 守卫 |
| `b173afd` | test(helix-im): E3 needs_continuation 续拉验收测试（4 场景）|

---

## §2 修复内容

### E4：scan_corr PortReply 路由（P0，功能死代码）

根因：on_start 吐 Persist{corr, Scan(channel_event_cursor)}，state.scan_corr = Some(corr)，
但 handle_port_reply 没有对应分支消费回报，scan 结果永远丢弃，proactive resync 实际不工作。

修复：
- handle_port_reply 新增路由 0：scan_corr == Some(corr) 命中时调 handle_scan_reply
- handle_scan_reply：按 engine_loop 约定的 [{"channel_id":N,"last_event_seq":N}] schema 反序列化，
  幂等注册 channel（entry().or_insert_with 不覆盖已有值），若已 Connected 立即发 proactive resync
- 降级：rows 解析失败 log warn + 不崩，空 channel 集启动仍可用

### E3：needs_continuation 续拉（P0，丢消息 bug）

根因：handle_sync_reply 的 Events { needs_continuation: _ } 用 _ 丢弃，
离线同步 >500 条截断不续拉 = 丢消息。

修复：
- Events 分支：若 needs_continuation=true，在 state.continuation_pending 注册 persist_corr → channel_id
- handle_port_reply 路由 1 persist ok：检查 continuation_pending 命中 → 调 maybe_continue_sync
- maybe_continue_sync：以推进后的 cursor 为 fromSeq 发续拉 Http；两路终止保证：
  1. events 为空但 continuation=true → 停止（服务端 bug 防御）
  2. cursor <= last_sync_from_seq → 停止（全 dup 幂等丢弃，cursor 未前进）

### B1：gate_inflight 守卫（顺带，架构评审待办 B1）

- Channel 新增 inflight_sync: Option<InflightSync> 字段
- 所有触发 sync Http 的路径（Connected proactive / gate timer / continuation）均设 inflight
- sync Http PortReply 到达时清 inflight
- Disconnected 清全部 inflight + corr_to_sync + continuation_pending（防重连时 B1 守卫误拦）
- emit_proactive_resync 抽取为共用方法，Connected 和 scan_reply 复用，消除重复

---

## §3 测试结果

```
cargo test --release -p helix-im

test result: ok.  4/4   replay.rs            PASS
test result: ok.  5/5   e4_scan_corr_test    PASS  <- E4 验收
test result: ok.  4/4   e3_continuation_test PASS  <- E3 验收
test result: ok.  9/9   gate_test            PASS
test result: ok.  9/9   lifecycle_test       PASS
test result: ok.  5/5   proptest_gate        PASS
test result: ok.  6/6   send_test            PASS
test result: ok.  3/3   stress_test          PASS

总计：45 passed / 0 failed（含既有 33 + 新增 9 + proptest 3）
```

---

## §4 改动文件

| 文件 | 净增/减 | 说明 |
|---|---|---|
| crates/helix-im/src/state.rs      | +14       | InflightSync newtype + continuation_pending HashMap |
| crates/helix-im/src/channel.rs    | +14       | inflight_sync + last_sync_from_seq 字段 |
| crates/helix-im/src/module.rs     | +221/-24  | E4+E3 实现 + B1 守卫 + Disconnected 清理 |
| crates/helix-im/tests/common.rs   | +38       | port_reply_with_body / sync_reply_events / scan_reply 辅助 |
| crates/helix-im/tests/e4_scan_corr_test.rs  | +146 | E4 验收 5 测试（新文件）|
| crates/helix-im/tests/e3_continuation_test.rs | +205 | E3 验收 4 测试（新文件）|

---

## §5 不变量声明

- 白名单约束遵守：只改 crates/helix-im/**，未触碰 helix-core / helix-driver-* / Cargo.toml / 根目录三文件
- 黑名单未触碰：SESSION.md / CLAUDE.md / README.md / docs/**
- HX-C001 验证：`cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 输出为空 PASS
