# Data Model: G-11 按 postId 定位

## GateScenario

| Field | Type | Rule |
|---|---|---|
| gateId | string | 固定为 `G-11` |
| actorId / observerId | string | canonical real lane 使用隔离双身份 |
| channelId | string | 每次 canonical run 由 CreateGroupPrerequisite 新建，不复用前一 Gate |
| command | tagged object | 仅允许 im_get_posts |
| correlation | object | reqId、transactionId、causationId 与业务 ID 可跨层关联 |
| expectedTerminal | enum | success、business-failed、infrastructure-failed、recovered |

## CanonicalFrameArtifact

| Field | Type | Rule |
|---|---|---|
| fixtureId | string | Gate 内唯一并表达独立可观察阶段 |
| schemaVersion | integer | 必须为 InteractionFrame V2 |
| surfaces | array | 仅包含 message.timeline |
| frame | object | 完整 envelope、scope、revision、mutations、bindings、directives |
| rawSha256 / semanticSha256 | string | Helix、PC、Mobile 三方一致 |
| producer | string | 当前 Helix source revision + 测试入口 |

## PlatformObservation

| Field | Type | Rule |
|---|---|---|
| platform | enum | pc-mock、pc-real-actor、pc-real-observer、mobile-gtest |
| frameApplied | object | transactionId 与 revision 必须匹配 |
| typedView | object | 只能由 SurfaceRuntime apply Frame 得到 |
| visibleState | object | PC DOM 或 Mobile QuickJS typed view |
| evidenceRefs | array | 原始日志、截图、report、hash 清单 |

## GateReview

| Field | Type | Rule |
|---|---|---|
| behavior | verdict | 权限、顺序、失败、恢复一致 |
| codeBoundary | verdict | Helix 唯一业务 reducer，测试能力 production-unreachable |
| frameV2 | verdict | 字段、surface、revision、mutation、action、directive 一致 |
| evidence | verdict | required pass=100%、fail=0、skip=0 |
| final | verdict | 四项全 Green 才可 review-green |

## State Transitions

`Draft -> HelixGreen -> PcGreen -> MobileGreen -> ReviewGreen`

- 任一 hash/pin/source 漂移：回到 `Draft`。
- 任一 required lane 缺失：进入 `Blocked`，不得解锁下一 Gate。
- Mock、AcceptedCommand 或 HTTP 2xx 不能直接进入 `PcGreen` 或 `ReviewGreen`。
