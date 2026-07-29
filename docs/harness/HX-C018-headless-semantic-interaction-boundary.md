---
id: HX-C018
title: Headless MessageV3 边界，业务事实只在 helix-im 计算
status: active
severity: blocker
scope: helix
created: 2026-07-15
updated: 2026-07-29
source: ADR-017 + Constitution 2.0 + Spec 047
recurrence_count: 2
trigger_paths:
  - "crates/helix-core/src/**"
  - "crates/helix-im/src/**"
  - "crates/helix-driver-*/src/**"
trigger_keywords:
  - "MessageV3"
  - "Effect::Emit"
  - "InteractionFrame"
  - "Surface"
  - "headless"
related:
  - HX-C001
  - HX-C007
  - HX-C010
graduates_to:
  - AGENTS.md
---

# HX-C018 — Headless MessageV3 业务边界

## §1 触发场景

新增或修改跨端业务 event、Gate 状态机、driver event delivery、平台 reducer，或讨论
“Headless 是否需要 Frame/Surface”时加载本卡。

## §2 当前决策

ADR-017 已取代 ADR-015。Headless 的当前含义是：`helix-im` 拥有平台无关的业务事实、
权限、未读、排序、恢复和完成条件，但只通过 `Effect::Emit {event,data}` 交付已提交结果。
driver 与 shell 不拥有第二套业务状态机，也不使用 Frame/Surface/Action/Directive 套壳。

## §3 Required / Forbidden

```rust
// helix-im：matching PersistOk 后构造唯一业务信封。
Effect::Emit {
    event: DomainEventBytes::from_json({
        "event": "im:post:received",
        "data": committed_post
    })
}
```

- durable event 必须晚于 matching `PersistOk`，并由本地事实或同一原子提交值构造。
- sender/receiver 的未读、角色、权限和摘要分别从各自 tenant-local 绝对态生成。
- Native channel、FFI batch 与 Web bridge 只原样交付 bytes。
- `helix-core` 不认识 IM schema；`helix-im` 不输出 DOM、Widget、CSS、像素或平台句柄。
- 禁止 InteractionFrame、SurfaceMutation、ActionBinding、UiDirective、FrameApplied。
- 禁止平台 raw event reducer、跨 event join、delta 补算或双写旧合同。
- 单 inbound 只产生有界 Effect 批次；禁止每 event 全表扫描形成 O(n²)。

## §4 Verification

```bash
# core 不认识 IM/UI schema。
! rg -n '\b(MessageV3Event|InteractionFrame|SurfaceMutation|ActionBinding|UiDirective)\b' \
  crates/helix-core/src

# 当前 Helix 生产与测试不保留退役业务合同。
! rg -n '\b(InteractionFrame|SurfaceMutation|ActionBinding|UiDirective|FrameApplied)\b' \
  crates/helix-im/src crates/helix-im/tests

# MessageV3 信封只能由 helix-im 生产路径产生。
rg -n 'Effect::Emit' crates/helix-im/src/event crates/helix-im/src/module
```

实现阶段还必须证明：PersistOk 前无 terminal Emit；匹配 reply 后恰好一次；重复、陈旧、
错误 scope 与 PersistErr 不发布成功；canonical JSON 来自真实 `ImModule::handle`。

## §5 Recurrence Log

| 日期 | 决策 | 现象 | 处理 |
|---|---|---|---|
| 2026-07-15 | ADR-015 | 试图用 Frame/Surface 统一呈现交互 | 建立旧语义交互合同 |
| 2026-07-29 | ADR-017 | 用户明确不要 Frame/Surface 套壳 | 改为 MessageV3 Gate Event 唯一真源 |

## §6 关联

- 北极星：`docs/GOAL.md`
- 决策：`docs/harness/decisions.md` ADR-017
- 规格：`specs/047-im-event-gate-inventory/spec.md`
- 兄弟护栏：HX-C001、HX-C007、HX-C010
