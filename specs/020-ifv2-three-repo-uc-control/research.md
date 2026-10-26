# Research: 26 Gate 三仓总控

## Decision 1

**Decision**: 一个 Gate 完成 Helix、PC、Mobile 与 Review 后再进入下一个。

**Rationale**: 并行实施会让共享生产文件和 Fixture pin 发生交叉漂移。

**Alternatives considered**: 26 Gate 并行、按仓横向批量实现；全部拒绝。

## Decision 2

**Decision**: G01 复用 019 基线；G02 是第一个试点；G03a、G04-G16c、GS1-GS4 依次推进。

**Rationale**: 符合用户确认的试点和纵向闭环策略。

**Alternatives considered**: GS 优先或按代码完成度跳序；除非用户改范围，否则拒绝。

## Decision 3

**Decision**: 26 Green 后只生成删除候选，用户选择后才删除并重跑。

**Rationale**: v1 名称可能属于非 Frame 版本域，机械删除不可接受。

**Alternatives considered**: 逐 Gate 物理删除、名称匹配全删；全部拒绝。

## Source Locks

- Helix: `918999d649007d92b085d8f5aba73329c9241ee4`
- cses-client: `13e42ef2793ea5c9f39b29f5d1ce86b1aca074db`
- Mobile: `674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21`
- 子 Feature: 25
