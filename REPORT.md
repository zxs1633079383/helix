# REPORT — 生命周期与网络 I/O 解耦根治（fix/lifecycle-net-decouple）

> worktree 线本地协调文件（§6.5）。EXPECT/DONE 落此，**不碰 dev-zlc 根 SESSION**；merge 时主对话汇总。

## 🎯 EXPECT 生命周期与网络 I/O 解耦根治（create 异步 connect + destroy 必达 Http 不阻塞）@2026-06-22 | 赌注 中 | 可逆 是（worktree 未 merge）

**根因（systematic-debugging 已确证 · instrumentation 实测）**：两个独立 30s（`DEFAULT_TIMEOUT_SECS=30`，连不上 `*.example.test`）：
- ① `pump.rs:110` `ws.connect().await` 阻塞 create 装配 30s（注释写「连不上仍进泵」但实现 await 了）。
- ② `engine.rs` graceful drain `http_pool(必达).shutdown().await` 无超时 join，等 im_send_message 的必达 `Http`(POST /posts/create) reqwest 30s timeout。

**可证伪断言**：
1. 连不上 host 时 `create+command+destroy` 总耗时 **< 3s**（当前 60s）——上界断言（HX-C011 允许）+ 结构化证（connect 是 spawn 任务、不在 create 同步路径）。
2. `cargo test -p helix-driver-ffi --test`/lib e2e `command_emits_domain_event_through_batch_callback` **< 5s**（当前 60s）。
3. native 零回归：`stress_engine` / `stress_storage` 仍过（共享 pump 核改动安全网）。
4. 真 Go 端到端（go+grpc 就绪）：**连得上时必达 Http 仍落地**（发消息成功、不丢）。
5. 五不变量⑤ 改写须同步 `engine.rs` 头注 + `helix-driver-native/CLAUDE.md`。
6. 只改 `helix-driver-{host,ffi}`（+ 可能 im outbox 兜底）；**core 零改**。

**范围界定**：根治 = 生命周期不被网络阻塞 + 乐观态(status=Sending)持久化兜底。**完整 outbox 重发可靠性**若超范围 → 记 backlog（不在本 worktree 强做）。

**偏离即记 ⚡ DRIFT-PRED**：若 connect 异步化破坏 `Effect::Send` 出站 / drain abort 丢必达消息且无兜底 / 改了 core / native 回归 / timing 断言 flaky（转结构化）。

## 文件白名单（只改这些）
- `crates/helix-driver-ffi/src/pump.rs`（connect 异步化）
- `crates/helix-driver-host/src/{engine.rs,network.rs,spawner.rs,pools.rs}`（drain 超时 + connect_timeout 基建）
- `crates/helix-driver-host/CLAUDE.md` / `helix-driver-native/CLAUDE.md`（五不变量⑤ 同步）
- 新增测试 `crates/helix-driver-ffi/tests/` 或 host tests（连不上快速退出回归）
- **黑名单**：core/** · 根 SESSION/CLAUDE/README · helix-im（除非 outbox 兜底必需，需先记 NEED）

## 进度
- [x] Phase 4.1 失败测试（连不上时 create/destroy 快速退出）—— `tests/lifecycle_net_decouple.rs` 3 用例
- [x] create connect 异步化（`pump.rs` spawn 后台连接，ready_tx 立即回传）
- [x] destroy 必达 Http drain 不阻塞（`spawner.rs::shutdown_with_timeout` + `engine.rs` HTTP_DRAIN_LIMIT=3s）
- [x] 五不变量⑤ 文档同步（engine.rs 头注 + host/native CLAUDE.md+AGENTS.md 镜像）
- [x] native 零回归（stress_engine 4 + stress_storage 2 全过）+ C++ ABI 冒烟 PASS

## 改的文件（白名单内，零黑名单触碰）
- `crates/helix-driver-ffi/src/pump.rs`（connect 异步化，+22/−6）
- `crates/helix-driver-host/src/engine.rs`（drain 上限 + 五不变量⑤ 头注，+30/−7）
- `crates/helix-driver-host/src/spawner.rs`（新增 `shutdown_with_timeout`；test mod 抽到 spawner_tests.rs 守 ≤300 硬顶）
- `crates/helix-driver-host/src/spawner_tests.rs`（新建——test mod 抽出，src ≤300）
- `crates/helix-driver-host/{CLAUDE,AGENTS}.md` + `crates/helix-driver-native/{CLAUDE,AGENTS}.md`（五不变量⑤ 镜像同步）
- `crates/helix-driver-ffi/tests/lifecycle_net_decouple.rs`（新建——3 上界回归 + ①② 各独立证伪）
- **黑名单零触碰**：`helix-core/**` 零改（cargo tree grep 空复证）· `helix-im/**` 零改 · 根 SESSION/CLAUDE/README 零改。

---

## ✅ DONE 生命周期与网络 I/O 解耦根治 @2026-06-22 | fix/lifecycle-net-decouple | create connect 异步化 + destroy 必达 Http drain 上限

### EXPECT 6 断言逐条对账

1. **连不上 host create+command+destroy < 3s** → ✅ 命中。实测 0.02s（`create_command_destroy_fast_when_unreachable`）。证伪：还原同步 connect → 5.02s FAIL。
2. **e2e `command_emits…` < 5s** → ✅ 命中。全 FFI 12 lib + 2 f5 + 3 lifecycle 用例 < 3.04s（drain 上限 3s 是最慢项，仍 < 5s）。
3. **native 零回归 stress_engine/stress_storage 仍过** → ✅ 命中。stress_engine 4/4（含 graceful_drain）+ stress_storage 2/2 全过。
4. **真 Go 端到端连得上时必达 Http 仍落地** → ⚠️ **部分**：本环境无真 Go 后端可连，**未做真连验证**（EXPECT #4 的「连得上」分支未覆盖）。已用结构化论证替代：drain 上限 3s 远大于 LAN 内 POST 往返（毫秒级），连得上时 worker 在限内完成 join → 走 `Ok(true)` 不 abort，落地路径不变（仅连不上才 abort）。C++ ABI 冒烟（release panic=abort）PASS 验证 optimistic emit 链路。
5. **五不变量⑤ 改写须同步 engine.rs 头注 + native CLAUDE.md** → ✅ 命中（含 AGENTS.md 镜像，4 文件一致 + structure-gate 镜像闸 PASS）。
6. **只改 helix-driver-{host,ffi}（+native 文档）；core 零改** → ✅ 命中。core 零改（cargo tree grep 空复证）；helix-im 零改（outbox 兜底**未触发 NEED**——见下「范围裁定」）。

### ⚡ DRIFT-PRED（实际偏离开局预期 → 候选教训）

- ⚡ **DRIFT-PRED ①（根因数值）**：开局 EXPECT 假设阻塞 = 30s（DEFAULT_TIMEOUT_SECS）。实测**本机阻塞 = 5s**——根因是 **TCP connect 层 OS timeout（~5s）先于 reqwest/tungstenite 的 30s 应用层 timeout 触发**。30s 仅在「SYN 被丢但 OS 不早停」环境出现。**根治正确性不受影响**（异步化/drain 上限对任意 timeout 值均成立），但「30s」是环境相关的，证伪阈值按 5s 标定。候选教训：**timeout 复现源要选「稳定 hang 的 blackhole IP（192.0.2.1）」而非「DNS 重定向 fast-refuse 的 *.example.test」**——后者本机 0.02s 假绿，测不出阻塞（已踩，改用 blackhole IP）。
- ⚡ **DRIFT-PRED ②（测试覆盖盲点）**：开局以为「总耗时 < 2s」单断言即覆盖 ①②。实测**单测只独立守 ①（create connect）**——destroy drain ② 因「command 的在途 Http 在 destroy 时未必挂起」而**默认测不出**（还原无上限 drain 仍假绿 0.02s）。补 `destroy_not_blocked_by_inflight_mandatory_http`：用录批回调等 `im:post:sending` emit 确保 Http 已 submit 到 worker（正卡 reqwest）后再 destroy，使 ② 独立可证伪。候选教训：**多根因 bug 的回归测试须逐根因独立证伪，不能靠一个聚合断言「碰巧覆盖」**（HX-C011 精神）。

### HX-C011 证伪证明（实测铁证）

- **① create 同步 connect 还原** → `create_*` FAIL @5.02s（应秒级）。根治 → PASS @0.02s。
- **② drain 无上限 `shutdown()` 还原** → `destroy_not_blocked_*` FAIL @4.99s（destroy 被在途必达 Http 阻塞）。根治 `shutdown_with_timeout(3s)` → PASS。
- 两处证伪互不串扰（①还原时 ② fix 在位、②还原时 ① fix 在位），证明两测各守一根因。

### 范围裁定（outbox 兜底不触发 NEED_IM_OUTBOX_FIX）

`pending_send.rs:13` + `on_timeout`：现网 source-of-truth 行为 = **15s 超时标 `send_status=unsend`，无自动补发**（resend 由用户/上层驱动）。destroy 超时 abort 在途 Http 的消息仍以 `status=sending` 留库（乐观态真源已落），与现网「sending/unsend 由用户重发」语义**一致**——非本 fix 引入的新缺口，**不需改 helix-im**。完整自动重发可靠性是独立 backlog（非本 worktree 范围），故**未写 NEED_IM_OUTBOX_FIX.md**。

### 验证命令 + 状态
- `cargo test -p helix-driver-ffi`：12 lib + 2 f5 + 3 lifecycle 全 PASS（< 3.04s）
- `cargo test -p helix-driver-native --test stress_engine --test stress_storage`：6/6 PASS
- `cargo test -p helix-driver-host --lib`：17 PASS（含 4 BoundedSpawner）
- `cargo clippy -p helix-driver-ffi -p helix-driver-host --all-targets`：零新 warning（FFI undocumented_unsafe_blocks deny 过）
- `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest'`：空（core 零运行时复证）
- `bash scripts/cpp-smoke.sh`：PASS @0.08s（release cdylib panic=abort，cpp→rust→cpp）
- `bash scripts/structure-gate.sh`：PASS（spawner.rs 167 行 ≤300；4 镜像一致）
