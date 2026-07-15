---
id: HX-C018
title: Headless 不是只吐 ViewModel，统一语义交互但禁止具体平台 UI 进入合同
status: active
severity: blocker
scope: helix
created: 2026-07-15
source: 2026-07-15 用户拍板 + dioxus-dev/crux-dev 双审查 + ADR-015
recurrence_count: 1
trigger_paths:
  - "crates/helix-core/src/**"
  - "crates/helix-im/src/**"
  - "crates/helix-driver-*/src/**"
  - "docs/specs/*interaction*"
trigger_keywords:
  - "ViewModel"
  - "ViewNode"
  - "ActionBinding"
  - "UiDirective"
  - "InteractionFrame"
  - "headless"
related:
  - HX-C001
  - HX-C007
  - HX-C010
graduates_to:
  - AGENTS.md
---

# HX-C018 — Headless 语义交互边界

## §1 触发场景

新增或修改多端 UI 投影、动作、directive、surface、frame、renderer key、FFI 批次，或讨论“Helix 是否只能输出 ViewModel”时加载本卡。

## §2 背景

2026-07-15 的旧方案把 Headless 收窄成“只输出数据 ViewModel”，并把 view tree/控件语义整体当成 Dioxus 路线禁止。dioxus-dev 与 crux-dev 双审查证明该边界仍会让 Angular、QuickJS/C++ 和未来 Flutter 各自解释动作、完成条件、未读、窗口与导航 fallback。用户据此拍板 ADR-015：Helix 必须统一平台无关的呈现状态机与交互语义，但仍不实现任何具体平台 UI。

## §3 Required / Forbidden

```rust
// ✅ helix-im：业务模块拥有平台无关的呈现与交互合同。
struct InteractionFrame {
    mutations: Vec<ViewMutation>,
    actions: Vec<ActionBinding>,
    directives: Vec<UiDirective>,
}

// render_kind 是语义 key，各端静态 registry 映射到具体组件。
// action_ref 是 opaque、surface-scoped 地址，UI 不知道 raw command。
```

```rust
// ❌ helix-core 不认识 IM/UI schema；helix-im 不输出具体平台实现。
enum CoreType {
    MessageRow(ViewNode),        // 禁止：业务 schema 泄漏进 core
    AngularComponent(String),    // 禁止：具体组件
    FlutterWidget(String),       // 禁止：具体 Widget
    CssSelector(String),         // 禁止：具体样式/DOM
}
```

- 必须把持久业务呈现状态、动作资格、fallback 和跨端 directive 放在 `helix-im`；各端只回送 `UiEvent/HostObservation/ack`，机械 apply/render/execute。
- 必须用 `surface_id + surface_epoch + revision` 隔离窗口/页面，旧 action/frame fail closed。
- 同一领域事实引起的多节点变化必须处于一个 `InteractionFrame`，FFI 按 frame 批量传输。
- `helix-core` 只提供不透明批次、端口与确定性机制，不 import 或公开 IM/UI 类型。
- 禁止把 DOM tag、Angular Component、Flutter Widget、CSS selector、像素、动画时长、平台句柄写进 Rust 交互合同。
- 禁止退回“只发 ViewModel，动作/未读/窗口/完成条件仍由各端解释”的半接管形态。
- action/node 索引使用 map 保持平均 O(1) 查找；一帧 apply 允许随 mutation/child 数量 O(n)，禁止每节点再全表扫描形成 O(n²)。

## §4 Verification

```bash
# core 必须不认识业务交互 schema。
! rg -n '\b(ViewNode|ViewMutation|ActionBinding|UiDirective|InteractionFrame)\b' crates/helix-core/src

# helix-im 合同不得绑定具体平台 UI。
! rg -n 'AngularComponent|FlutterWidget|DOM tag|CSS selector|pixel position|animation duration' crates/helix-im/src

# 文档真源必须明确 Headless 不是 ViewModel-only，并明确具体 UI 红线。
rg -n 'Headless 不是.*ViewModel|Headless 交互内核' docs/GOAL.md
rg -n 'InteractionFrame.*ViewMutation.*ActionBinding.*UiDirective' docs/GOAL.md docs/specs/*interaction*
```

实现阶段还必须增加：相同输入剧本的 native/FFI frame 字节等价测试、revision gap/resync 测试、stale action fail-closed 测试，以及破坏 Helix projector 后 PC/Mobile 同时失败的负向测试。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-07-15 | ADR-015 / 本会话 | 新 InteractionProtocol 一度被旧 GOAL“只允许数据 ViewModel”反向否决 | 把 Crux 的数据投影下限误当成 Helix Headless 的能力上限 |

## §6 关联

- 北极星：`docs/GOAL.md` §1.8 / §6
- 决策：`docs/harness/decisions.md` ADR-015
- 合同：`docs/specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md`
- 机制参照：Dioxus 稳定事件地址/批量 mutation；Crux Model/update/view/Effect 分层
- 兄弟护栏：HX-C001（core 零平台依赖）、HX-C007（跨 FFI 批处理）、HX-C010（确定性顺序）

## §7 历史与演进

- 2026-07-15：用户明确拍板并覆盖旧 ViewModel-only 边界，直接进入 active。
