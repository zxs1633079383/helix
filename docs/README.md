# Helix 文档地图

> `docs/` 的单一入口。本文只维护 canonical 路由、证据层级和熵管理规则，不复制任务
> 流水或 Green 数字。
>
> 最近代码对账：2026-07-27。对账范围：Helix、cses-client、mobile-qucik-c++ 当前
> HEAD 与消费 pin、ADR-014/015/016、Strict Matrix epoch 11、Spec 018/020。

---

## 1. 冲突裁决顺序

| 层级 | 真源 | 负责回答 |
|---|---|---|
| L0 | 当前代码、配置、消费 pin、运行 artifact | 现在实际执行什么 |
| L1 | Strict Matrix + machine evidence pack | 哪个 Gate 在哪些证据面完成 |
| L2 | [`GOAL.md`](./GOAL.md) | 为什么做、不可违反什么、TSCR 如何计算 |
| L3 | [`harness/decisions.md`](./harness/decisions.md) | 已拍板的 ADR 与边界 |
| L4 | Spec/Plan/Tasks | 某次 Feature 的范围、顺序、完成合同 |
| L5 | [`ROADMAP.md`](./ROADMAP.md) | 当前优先级、阻塞和下一步 |
| L6 | [`ROADMAP-full.md`](./ROADMAP-full.md) | 从地基到三端产品闭环的战略全景 |
| 历史 | 设计推演、旧报告、旧 pin 证据 | 解释来源，不证明当前状态 |

规则：

- 代码事实冲突时，代码优先；目标冲突时，GOAL/ADR 优先。
- `declared`、`implemented`、fixture pass、contract-green、Mock、real、dual-user、
  persistence、UI 和 TSCR 是不同状态，禁止互相替代。
- 旧 evidence pack 只有在 revision、Matrix hash 和证据窗口仍匹配时才能参与当前裁决。

---

## 2. 必读链

### 每次会话

1. 根 [`AGENTS.md`](../AGENTS.md)
2. [`GOAL.md`](./GOAL.md)
3. 根 [`SESSION.md`](../SESSION.md)
4. [`harness/README.md`](./harness/README.md)
5. 与任务对应的 crate-local `AGENTS.md`

### 做进度或发布判断

1. [`crates/helix-im/contracts/strict-matrix.v1.json`](../crates/helix-im/contracts/strict-matrix.v1.json)
2. [`crates/helix-im/docs/Strict Matrix/README.md`](../crates/helix-im/docs/Strict%20Matrix/README.md)
3. 对应 Spec 的 `verification.md` 与 machine evidence
4. PC/Mobile/Web 当前消费 pin 与运行报告
5. [`ROADMAP.md`](./ROADMAP.md)

---

## 3. 当前架构入口

| 主题 | Canonical 文档 | 说明 |
|---|---|---|
| 北极星与 TSCR | [`GOAL.md`](./GOAL.md) | 运行时无关、三端同核、唯一北极星指标 |
| 当前活账 | [`ROADMAP.md`](./ROADMAP.md) | 状态分面、P0/P1/P2、停止条件 |
| 战略全景 | [`ROADMAP-full.md`](./ROADMAP-full.md) | 地基、V2 协议、三端消费、全量能力阶段 |
| ADR | [`harness/decisions.md`](./harness/decisions.md) | ADR-001..016；ADR-014/015/016 为当前三端、交互、媒体边界 |
| IM 机制索引 | [`crates/helix-im/docs/README.md`](../crates/helix-im/docs/README.md) | IM 合同、设计、Strict Matrix 与迁移文档入口 |
| 源码阅读 | `crates/helix-{core,im,driver-host,driver-native,driver-ffi,driver-web}` | 以当前 trait、调用链和 crate-local `AGENTS.md` 为准 |
| 可执行护栏 | [`harness/README.md`](./harness/README.md) | HX-C001..HX-C018 |

三层代码边界：

```text
helix-im / future modules
        ↓ only core ports
helix-core
        ↑ adapter implements ports
helix-driver-host / native / ffi / web
        ↑ protocol conversion only
PC Angular / Mobile QuickJS-Flutter / Web renderer
```

---

## 4. InteractionProtocol 入口

| 文档/目录 | 角色 | 能否证明当前 Green |
|---|---|---|
| [`specs/018-interaction-frame-v2-cutover/`](../specs/018-interaction-frame-v2-cutover/) | V2-only writer/reader、V1 清退、25 Gate 硬切合同 | 只有 verification/evidence 对应的固定 candidate 可以 |
| [`specs/020-ifv2-three-repo-uc-control/`](../specs/020-ifv2-three-repo-uc-control/) | Helix + PC + Mobile 串行 Gate 总控 | 子 Gate review-green 不能自动变成 TSCR |
| [`docs/specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md`](./specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md) | ADR-015 的三端语义交互目标合同 | 不能，属于目标 Spec |
| [`crates/helix-im/docs/interaction-v2/`](../crates/helix-im/docs/interaction-v2/) | G01-G16c Frame/Surface/Action 时序与数据 | 不能，需绑定 producer 与 runtime evidence |
| [`crates/helix-im/tests/fixtures/`](../crates/helix-im/tests/fixtures/) | canonical Frame V2 与 provenance | 只能证明 producer/合同面 |
| [`crates/helix-im/docs/Strict Matrix/`](../crates/helix-im/docs/Strict%20Matrix/) | 46 Gate 合同与八面证据规则 | Matrix 自身不运行测试 |

协议版本必须分开：

- `InteractionFrameV2.schemaVersion = 2`
- `ActionInvocationEnvelope.schemaVersion = 1`
- `strict-matrix.v1.json` 的 `v1` 是 Matrix 文件格式

它们不是同一版本轴。

---

## 5. 三端事实入口

### Helix

- Frame 合同：`crates/helix-im/src/interaction_frame.rs`
- 统一 Surface compose：`crates/helix-im/src/projector/`、`surface_runtime.rs`
- host 执行：`crates/helix-driver-host/`
- native/FFI/Web adapter：`crates/helix-driver-{native,ffi,web}/`
- 当前 Matrix：epoch 11，46 Gate，其中 43 active、3 excluded
- 当前 canonical fixture：22 个 provenance 目录、104 个 Frame V2 文件

### PC / cses-client

- Tauri host：`src-tauri/src/features/helix_im/`
- Angular consumer：`src/pages/message-v4/surface-runtime/`
- 当前 Cargo pin：Helix `98c522a3621cc599c134cb78b9e4656668eb67bb`
- current-epoch runtime 判断必须读取最新 reducer/evidence pack；历史 `canonical-green=26`
  与“待按新 pin 重绑”的记录不能同时被当作当前 TSCR 证据

### Mobile / mobile-qucik-c++

- source lock：`contract/helix-source.lock.json`
- FFI/SDK/SurfaceRuntime：`cpp/`、`venus/`
- 当前 Helix pin：`98c522a3621cc599c134cb78b9e4656668eb67bb`
- `27/27 mobile contract-green` 只证明 command/event 路由合同
- 2026-07-16 的 real-chain 报告基于旧 Helix `1d80c90`、Matrix epoch 7，且不含
  Android/iOS/Flutter 真机 UI；它是历史证据，不是当前 TSCR

### Web

- Rust 平台 driver：`crates/helix-driver-web/`
- browser adapter/smoke：`crates/helix-driver-web/browser/`
- 已有 final WASM、Browser Fetch、session store、WS bridge 与真 Go smoke
- crate 明确“不包含 frontend rendering”；产品 renderer、全 Gate action/apply/ack 和
  UI evidence 仍是 TSCR 的缺失面

---

## 6. 证据语言

| 状态 | 含义 |
|---|---|
| `declared` | Matrix 声明了合同 |
| `implemented` | 当前代码实现了合同，仍不表示运行完成 |
| `fixture-pass` | canonical producer/codec/replay 通过 |
| `contract-green` | 某消费端 command/event/ABI/类型合同完整 |
| `mock-pass` | 不访问真实 authority 的 UI/driver 测试通过 |
| `real-pass` | 指定端的真实 I/O 链通过 |
| `dual-user-pass` | actor/observer 同窗完成 |
| `persistence-pass` | local/remote authority 与 Persist barrier 可关联 |
| `ui-pass` | 产品 UI 的最终状态和交互可观察 |
| `ThreePlatformGreen` | GOAL §5 的全部条件同时满足 |

只有 `ThreePlatformGreen` 进入 TSCR 分子。任何报告使用 `Green` 时必须带完整限定词。

---

## 7. 其他领域入口

| 主题 | Canonical 文档 |
|---|---|
| UC command/projection/HTTP/WS/storage 综合映射 | [`crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](../crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md) |
| 投影 schema | [`crates/helix-im/docs/projection-schema.md`](../crates/helix-im/docs/projection-schema.md) |
| 真机 curl | [`crates/helix-im/docs/csesapi-migration/真机curl真源.md`](../crates/helix-im/docs/csesapi-migration/真机curl真源.md) |
| API 全量图谱 | [`crates/helix-im/docs/csesapi-migration/full-map/00-README-index.md`](../crates/helix-im/docs/csesapi-migration/full-map/00-README-index.md) |
| FFI 合同 | [`crates/helix-driver-ffi/docs/`](../crates/helix-driver-ffi/docs/) |
| Helix Runtime Metrics | [`Metrics/Prometheus指标说明.md`](./Metrics/Prometheus指标说明.md) | 当前分支 116 个 Runtime 指标的分类、语义、OTLP/Prometheus 链路和验收查询 |
| 性能实测 | [`reports/helix-并发承压评估-2026-06-11.md`](./reports/helix-并发承压评估-2026-06-11.md) |
| 提交规范 | [`COMMIT_CONVENTION.md`](./COMMIT_CONVENTION.md) |

---

## 8. 熵管理

1. **一事实一真源**：目标只进 GOAL，决策只进 ADR，合同只进 Matrix/Spec，状态只进
   evidence/ROADMAP，历史只做链接。
2. **不复制动态数字**：除带 snapshot 日期、revision、epoch、hash 的对账段外，不在叙事
   文档复制 Green 数量。
3. **过期即降级**：旧 pin、旧 epoch、旧运行窗口的报告标题或开头必须标记“历史证据”，
   不继续作为当前入口。
4. **目标与现状并列**：Spec 中的 TARGET、代码 CURRENT、未兑现 GAP 必须明确分栏。
5. **删除优于堆叠**：被 ADR 覆盖的路线从 active roadmap 删除；若仍有解释价值，压缩成
   “历史决策”链接，不保留两套执行步骤。
6. **新文档准入**：新建 Markdown 前先证明无法进入现有 canonical 文件；否则更新原文。
7. **入口同步**：新增/移动/删除 canonical 文档时同步本文、局部 README 与引用检查。
8. **断链与漂移检查**：提交前检查 Markdown link、ADR/HX-C 数量、Matrix gate 数、旧术语、
   `git diff --check`。
9. **证据不过夜升级**：实现提交可以先于真实证据，但 ROADMAP 只能写精确分面；
   `partial`/`blocked` 不得等待“下次整理”才补。
10. **TSCR 单一出口**：任何周报、Roadmap 或发布稿都引用 GOAL §5 的机器派生值，不另造
    “总体完成度”。
