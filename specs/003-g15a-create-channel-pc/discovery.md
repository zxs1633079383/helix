# Source Discovery

**Feature**: `specs/003-g15a-create-channel-pc`
**Source Lock**: `b4339e252e865ea294c51001388b1ef0de3ba42e`
**Worktree State**: `dirty; pre-existing user changes preserved`

## Facts *(mandatory)*

- **F-001**: G15a 的目标合同把建群定义为会话列表的稳定 capability，PC 输入过程留在平台 overlay，最终规范化请求进入共享业务链。 | **Evidence**: `crates/helix-im/docs/interaction-v2/08-G15-G16c-建群话题与频道治理.md:7`
- **F-002**: 目标链要求权威频道事实提交后才形成会话列表更新，创建者进入频道前必须取得同频道的摘要与消息输入 bundle。 | **Evidence**: `crates/helix-im/docs/interaction-v2/08-G15-G16c-建群话题与频道治理.md:10`
- **F-003**: 当前 PC 建群组件收集群名称和成员，锁定当前用户，拒绝空名称、超过 30 字符或没有其他成员的提交，并把群类型固定为私有群聊。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-create-chat.component.ts:23`
- **F-004**: 当前 PC shell 将最终草稿收敛为 type、user_ids、display_name 和 force_create 四个共享命令输入。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/shell.service.ts:199`
- **F-005**: Helix 当前建群 outbound 边界只接受四个 canonical 字段，拒绝额外字段，并把创建者身份、team、成员角色和默认头像留给可信服务端会话派生。 | **Evidence**: `crates/helix-im/src/outbound/channel_admin/existing.rs:48`
- **F-006**: 当前命令路径会保存本次规范成员集合的关联上下文，以便权威创建结果回到同一建群请求。 | **Evidence**: `crates/helix-im/src/module/trait_impl.rs:207`
- **F-007**: 当前 HTTP 创建结果会校验成功信封和合法频道身份，补齐规范成员投影，并尝试更新已挂载的会话列表。 | **Evidence**: `crates/helix-im/src/port_reply/dispatch_channel_create.rs:25`
- **F-008**: 当前实时 channel_created 路径对合法频道执行频道/成员写入，并在会话列表已挂载时产生插入更新；这需要在 Plan 阶段与 PersistOk 后发布的目标顺序对账。 | **Evidence**: `crates/helix-im/src/ws/handlers/channel_created.rs:36`

## Unknowns

- **U-001 RESOLVED**: 当前 HTTP 与实时两条路径都会先生成多个 storage effect，再在同一 tick patch Frame；Effect 向量不存在“持久化完成后可见”的保证。 | **Resolution**: 统一 `PersistAtomic`，只在匹配 `PersistOk` 后发布 Frame；见 `research.md` R-003。
- **U-002 RESOLVED**: 受邀成员必须在自己的 Helix session/user scope 基于权威事实形成结果；旧 raw emit 不能证明 typed Frame 已独立收敛。 | **Resolution**: 增加双用户 replay 与真链证据；见 `research.md` R-004。

## Decisions

- **D-001**: 本 Spec 的用户交付面只包含 PC；移动端和 Web 专属界面不进入任务与完成证据。
- **D-002**: 本 Spec 只定义私有群聊创建，不把私聊、开放频道、话题或治理动作混入 G15a。
- **D-003**: 传输成功不是完成；完成必须包含权威频道事实、本地频道/成员事实和创建者唯一可见群聊结果。
