# Verification Procedure: G-15a

## Source Lock

- Helix 最终实现 HEAD：`91f6b0283566e25a8e8b77a5894b9bc6e181bdd8`；canonical Fixture producer：`157daca3760e9c94d7b6bc9f38798e199628b1aa`。
- cses-client 最终实现 HEAD：`01ee73c430c7ab39ab14411b4d0eae7f8b361950`；production Tauri Helix pin：`157daca3760e9c94d7b6bc9f38798e199628b1aa`。
- Mobile 最终实现 HEAD：`afa41cd8d2dd6567a42d258c3ba504a0210f1125`；production Helix source pin：`91f6b0283566e25a8e8b77a5894b9bc6e181bdd8`。
- Strict Matrix SHA-256：`8b551c0d7aead1e44bbba355f65d65f17c883d84fbb2090854f0942bb4904fb7`。
- 前序 G-14a：`specs/040-ifv2-g-14a-template-received/review-ledger.json` 为 `phase=closed`，R1-R4 全 PASS。
- 工作树边界：PC clean；Mobile 只保留用户既有未跟踪 bundle 与 `tests/.DS_Store`；Helix 只含当前 Spec/SESSION/evidence 状态。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-15a` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 最终关闭以 `review-ledger.json` 的 R4 机器 verdict 为准；本文件不能自行宣称 `CLOSED`。

## Helix Producer Green

- `cargo check -p helix-im`：退出码 0；`cargo tree -p helix-core` 运行时依赖扫描为空。
- `g15a_full_flow_test`：6/6 PASS，覆盖业务失败、基础设施失败、持久失败重放、权威竞态幂等、happy path 与 summary/roster recovery。
- 生产顺序为 `im_create_channel -> HTTP admission -> authoritative channel/member fact -> PersistAtomic -> PersistOk -> Effect::Emit`；HTTP 2xx 不直接发布成功 Frame。
- `g15a_wdio_fixture_contract_test`：1/1 PASS；普通模式只比较 canonical Golden，三份 Frame 均由 `G15aScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode` 产生。

## PC Consumer Green

- PC implementation HEAD 为 `01ee73c4...`；Fixture、typed loader、TauriMockBridge、PageObject、Mock/Real/report 与 boundary 均归测试边界，production bundle 扫描 `mockProductionReachable=false`。
- `message:wdio:g15a:mock`：2/2 PASS；`message:wdio:g15a:report`：1/1 PASS；Mock 只模拟同步 `AcceptedCommand`，完整 Frame 经真实 Angular `SurfaceRuntime` apply 后再断言 DOM。
- `message:wdio:g15a:single-real`：1/1 PASS；从真实 MessageV4 UI 发起 `im_create_channel`，观察 Helix/SQLite/HTTP/WS、FrameApplied 与最终 DOM。
- `message:wdio:g15a:dual-real`：最终 1/1 PASS；Actor 444 与 Observer 678 共享同一服务端 `channelId`，分别收敛为 `owner`/`member`，summary、roster、本地持久与远端 PG 权威一致。
- 首次 dual-real 在 UCenter 成员目录中等待 `678` 超时，第二次事件式自动等待通过；失败原文与最终 PASS 均保留在 `V-005-pc-real.log`，不以静默重跑覆盖。
- WDIO 启动日志中的 `tauri-driver not found` 是 embedded WebKit fallback 的非致命诊断；实际 single-real 与 multiremote session 均已运行并通过。

## Mobile Consumer Green

- Mobile implementation HEAD 为 `afa41cd8...`；三份 Fixture byte-exact mirror，production Helix pin 为 `91f6b02...`。
- `G15AFrameContractTest` 与 UC15 聚焦合同合计 15/15 PASS，覆盖完整 envelope、双 surface 原子 apply、恢复快照、错误整帧拒绝、DomainEvent batch、production C ABI 与 QuickJS typed view。
- `run-uc01-16-gtest.sh --only UC15` 的首轮真实链为 10/10 PASS；先等待 typed dialog，再创建 fresh group，从 Frame V2 取得权威 `channelId`，随后验证 typed summary/roster、双用户、`viewerRole=owner` 与 `cursor=initial`。
- 共享 prerequisite 对已挂载的 G15a 强制使用 typed dialog；未迁移 UC 的测试态 raw-event fallback 仍由显式测试路径保留，不进入 production build。
- source pin、legacy hard-cut、no-JS-facade 与共享 Venus QuickJS smoke 均 PASS；G15a 未修改 Venus/QuickJS 接线能力，因此不新增 Gate 专属 Venus runner。
- 额外 Mobile 全 UC shared real regression 暴露 GS01/GS02/GS03 旧前置仍等待已硬切断的 `im:channel:created`，随后远端建群也出现超时；原始失败保留在 `V-008-mobile-shared-regression-blocked.log`。该 lane 按总控合同为 non-required regression，不能替代也不否定生产链接的 Mobile Contract Green；后续对应 GS Gate 必须迁到 typed CreateGroupPrerequisite 后才能各自 Green。

## Cross-repo Fixture Parity

- 三仓 inventory 均为三份 Frame + provenance，逐文件 `cmp` 通过，schemaVersion 全为 2。
- `channel-created.frame-v2.json`：raw `7af79514d959c45dd21c74a2f2980917ae7486da7a82b15cc2d6e4a5a4a42631`，semantic `d3919a4f7f61ffbc0d8cfa5bac74067f5e6176afc07065ab416835ba099f87da`。
- `channel-summary-recovery.frame-v2.json`：raw `03e3b59aca0cc1148cad2cf02988ac750a2f884b33d07ba4e54c6aaec3a86845`，semantic `c7ec217f0e2db2e7f2fb72774b1df6a07685a95f7a8f4ed097bccd7ad643ecad`。
- `member-roster-recovery.frame-v2.json`：raw `07059b78824f9a55186b7dd89321442559226a659ad6f3d26338f43edacabb6a`，semantic `a6fc893a5ed1d4f9c8526c8fe821b2d0b12e293c4baf4dec847c6d2f44e9ff7d`。
- `provenance.json` 三仓 raw SHA-256 均为 `7bb1d819a500070d1965cf770bdabb065fb6453aeec355a78479d964c3c77fb4`。

## Review

### Behavior

- **Green**：每次 canonical 验收创建 fresh two-user group，不依赖前一 Gate 遗留数据；权限、业务失败、基础设施失败、持久屏障、竞态幂等与 cold recovery 均有独立断言。
- **Green**：Actor/Observer 最终都看到同一频道和两名成员；角色按身份分别为 owner/member，终态不由 PC 或 Mobile 推算。

### Code Boundary

- **Green**：业务 reducer 唯一真源为 production `ImModule::handle`；ScenarioRunner 只注入确定性 Tick/PortReply/Inbound 并捕获 Emit。
- **Green**：PC Fixture/Mock/WDIO API 与 Mobile 测试注入均 production-unreachable；Mobile GTest 链接 production SDK/Bridge/FFI/SurfaceRuntime/QuickJS binding。
- **Green**：Mobile production V1 producer=0、consumer=0；测试态 legacy characterization 不冒充本 Gate 生产合同。

### Frame V2

- **Green**：创建终态 Frame 同批包含 `message.channel-summary` 与 `message.member-roster`；两者共享 transaction/correlation，schemaVersion=2、surfaceEpoch=1、revision=1、baseRevision=null。
- **Green**：独立 recovery 均为 revision 1 snapshot；roster scope 使用 `cursor=initial`，不再混用 channel scope；summary 显式 resync 总是返回快照。
- **Green**：PC/Mobile 不补业务字段、不重排 mutation；unknown schema/surface/revision gap 均整帧 fail-closed。

## Evidence Manifest

- `evidence/evidence.json` 以 SHA-256 登记 V-001～V-007；Helix、Fixture、PC Mock、PC Real、Mobile 与 Review 证据彼此独立。额外 non-required shared regression 原文单独保留为 V-008，不计入 required PASS。
- `evidence/gate-report.json` 按机器 schema 汇总所有 required lane；最终关闭仍须 mandatory `after_implement` loop 给出 R4 PASS。
