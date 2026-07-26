# Implementation Plan: G-15 创建话题 三仓纵向闭环

**Branch**: `042-ifv2-g-15-create-topic` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

**Input**: [research.md](research.md)、[data-model.md](data-model.md)、[contracts/gate-acceptance.md](contracts/gate-acceptance.md) 与 Strict Matrix 中 `G-15` 的当前合同。

## Summary

以 Helix 的 `module/trait_impl.rs -> port_reply/dispatch_projection.rs` make-topic 生产链为业务真源，并复用 `channel/gate/` 的独立 WS authority，补齐 `G-15` 的完整 sans-IO 场景、字段级 InteractionFrame V2、canonical Fixture/Golden/provenance；PC 只读同步 Fixture 后分别完成 WDIO Mock、真实 Tauri/Helix 链、DOM 与 report；Mobile byte-exact 同步同一 Fixture，通过生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest 验证 typed view。三仓完成后执行行为、代码边界与 Frame V2 Review，只有 review-green 才解锁 `下一 Gate`。

## Technical Context

**Language/Version**: Rust workspace 当前工具链；TypeScript 5.6/Angular 20.3/Node 20；C++17/QuickJS/GoogleTest

**Primary Dependencies**: helix-core ports、helix-im `ImModule::handle`/InteractionFrameV2；cses-client TauriMockBridge/SurfaceRuntime/WebdriverIO；MobileImSdk/MobileFfiBridge/SurfaceRuntime/HelixImQuickJsBinding

**Storage**: Helix SQLite StorageOp（channel、message；typed thread Frame 取代平台 raw reply_tree reducer）；远端 PostgreSQL `channel、channel_member、channel_topic、post、channel_event` 只作有界证据

**Testing**: Rust deterministic replay + Golden；WDIO Mock/single-real/dual-real + reporter；C++ GTest + QuickJS typed view

**Target Platform**: Helix native/FFI 共用业务核；PC Tauri WKWebView；Mobile C++/QuickJS

**Project Type**: 三仓纵向 UC 交付，Helix domain/application，PC/Mobile platform adapter

**Performance Goals**: core 每 Tick 有界；Frame apply 为 O(k mutations)；fixture parity 为 O(n files)；等待使用 correlation latch，不固定 sleep

**Constraints**: Helix 是唯一业务 reducer；Fixture/Mock/GTest 不补业务字段；测试入口默认 production build 不可达；外部输入零 unwrap/expect；FFI batch 不逐条 callback

**Scale/Scope**: 单一 `G-15`；command=im_make_topic；effect=http_write、storage_upsert、projection；surface=message.channel-summary、message.thread-detail。

## Constitution Check

| 原则 | 设计判定 |
|---|---|
| Runtime-Agnostic Layering | PASS：业务改动只在 helix-im，PC/Mobile 只实现 adapter 与 renderer |
| Deterministic Facts | PASS：时间、ID、PortReply、Inbound 全由 ScenarioRunner 注入 |
| InteractionFrame V2 Truth | PASS：PC/Mobile 不消费 raw business event 重建状态 |
| Evidence Before Completion | PASS：Helix、Fixture、Mock、Real、Mobile 与 Review 分层 |
| Minimal Governed Migration | PASS：只处理 G-15，不带入 G03 或 G17-G27 |
| Spec Kit Admission | PASS：R1 已通过；R2/R3 后才允许实现 |
| 热路径与 FFI | PASS：Frame O(k) apply、batch crossing、无无界扫描 |

**Post-design re-check**: PASS。contracts 只冻结 Gate 验收与报告，不增加第二套业务协议；平台 mirror 只读；真实删除仍由 26 Gate 总控和用户决定。

## Loop Detail Contract

### Inputs and Context

- **I-001**: [discovery.md](discovery.md) 的三仓 source lock、当前工作树边界与代码事实。
- **I-002**: Strict Matrix 的 command/effect/persistence/projection/HTTP/WS/remote-store 合同。
- **I-003**: `crates/helix-im/tests/g15_topic_interaction_test.rs`、`crates/helix-im/src/module/trait_impl.rs` 与 `crates/helix-im/src/port_reply/dispatch_projection.rs` 的当前生产行为。
- **I-004**: G01 canonical Fixture、PC WDIO Mock/Real/report 与 Mobile GTest/QuickJS 基线。
- **I-005**: [contracts/gate-acceptance.md](contracts/gate-acceptance.md)、两个 JSON Schema 与 [quickstart.md](quickstart.md)。

### Action Chain

- **A-001**: fresh group prerequisite -> im_make_topic command/observation -> `ImModule::handle` -> ordered Effects。
- **A-002**: Persist/HTTP/WS PortReply 或 Inbound -> terminal authority -> SurfaceComposer -> complete InteractionFrame V2。
- **A-003**: Effect::Emit -> typed decode/assert -> `crates/helix-im/tests/fixtures/g15-wdio/` Golden/provenance。
- **A-004**: Helix Fixture -> PC mirror/hash -> TauriMockBridge bus replay -> Angular SurfaceRuntime -> DOM/report。
- **A-005**: 真实 MessageV4 UI -> Tauri invoke -> Helix/SQLite/HTTP/WS -> FrameApplied -> Actor/Observer DOM。
- **A-006**: Helix Fixture -> Mobile mirror -> production SDK/FFI DomainEvent batch -> SurfaceRuntime atomic apply -> QuickJS `im.surface.view`。
- **A-007**: 三仓证据 -> behavior/code/Frame V2 review -> review-green 或 fail-closed blocker。

### State Contract

- **S-001**: `Draft -> HelixGreen`：完整场景、生产 Frame、Golden 与 provenance 全通过。
- **S-002**: `HelixGreen -> PcGreen`：Mock 与 required real lane、DOM、FrameApplied、report 全通过。
- **S-003**: `PcGreen -> MobileGreen`：mirror parity、生产链 GTest、QuickJS typed view 与 hard-cut 全通过。
- **S-004**: `MobileGreen -> ReviewGreen`：行为、代码边界、Frame V2 与 evidence Review 全 Green。
- **S-005**: 任一 pin/hash/schema/source 漂移 -> `Draft`；required 环境缺失 -> `Blocked`。

### Validators

- **V-001**: `cargo check -p helix-im` 与 `cargo test -p helix-im --test g15_full_flow_test -- --nocapture` 证明完整确定性流程。
- **V-002**: `cargo test -p helix-im --test g15_wdio_fixture_contract_test -- --nocapture` 证明完整 Frame、Golden 与 provenance。
- **V-003**: `yarn message:wdio:g15:mock` 证明 PC mirror/Mock/DOM/report，不提升为 real。
- **V-004**: `yarn message:wdio:g15:single-real` 与 required dual lane 证明真实 Tauri/Helix/WS/DOM。
- **V-005**: `./scripts/run-gtest-js-spec.sh surface-runtime --gtest-filter='G15FrameContractTest.*'` 证明 Mobile production consumer 与 QuickJS typed view。
- **V-006**: production-boundary/source scans 证明 Fixture、Mock、测试 ABI、raw reducer 在默认构建不可达。
- **V-007**: cross-repo review 对齐 command、terminal、Frame、hash、DOM、typed view 与报告，输出 review-green 或 blocker。

## Project Structure

### Documentation

```text
specs/042-ifv2-g-15-create-topic/
├── spec.md
├── discovery.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── verification.md
├── contracts/
├── review-ledger.json
└── tasks.md
```

### Source Code

```text
crates/helix-im/src/module/trait_impl.rs
crates/helix-im/src/port_reply/dispatch_projection.rs
crates/helix-im/src/channel/gate/
crates/helix-im/tests/g15_topic_interaction_test.rs
crates/helix-im/tests/g15_full_flow_test.rs
crates/helix-im/tests/g15_wdio_fixture_contract_test.rs
crates/helix-im/tests/support/g15_scenario.rs
crates/helix-im/tests/fixtures/g15-wdio/

/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/fixtures/g15/
/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/pageobjects/g15.page.ts
/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g15-mock.e2e.ts
/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g15-single-real.e2e.ts
/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g15-dual-real.e2e.ts
/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/reporters/g15-summary.ts

/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc15ChannelCreateTest.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/G15FrameContractTest.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/fixtures/g15/
/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileFfiBridge.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/HelixImQuickJsBinding.cpp
```

**Structure Decision**: 不建立平台业务 Store；Helix 测试与 Fixture 属 producer，PC/Mobile 文件只验证或机械 apply 同一 Frame。现有共享 harness 优先扩展，Gate 文件只承载业务特定场景和断言。

## Delivery Phases

1. Source revalidation 与 fresh-group prerequisite。
2. Helix RED 场景、生产 handler/persist/projector、Fixture/Golden。
3. PC mirror、Mock、single/dual real、DOM/report 与 production-boundary。
4. Mobile mirror、GTest、SDK/Bridge/SurfaceRuntime/QuickJS typed view 与 hard-cut。
5. 行为、代码、Frame V2、evidence Review 与 R4。

## Complexity Tracking

无 Constitution 例外。任何需要平台业务 reducer、producer dual-write、raw fallback 或测试注入进入生产的方案直接拒绝，不登记为可接受复杂度。
