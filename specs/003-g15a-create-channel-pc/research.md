# Phase 0 Research: G15a PC 创建群聊

## R-001 创建入口属于 DialogList 根级 capability

**Decision**: 在 `DialogListVm` 增加根级 `actions: Vec<ActionBinding>`，注册 `channel.create`；不把建群动作塞进任意 dialog row。

**Rationale**: G15a 是“创建新会话”，不存在可归属的既有 row。当前 `DialogListVm` 没有根级 actions，而 `DialogRowVm.actions` 面向已有频道动作。

**Alternatives rejected**: 复用旧 PC direct `shell.createChat()` 会绕开 ActionBinding 的 actor/scope/epoch/revision；伪造一行“创建群聊”会污染业务列表数据模型。

## R-002 PC overlay 是本地输入过程，不是共享业务事实

**Decision**: 复用并接入现有 `MessageUiCreateChatComponent` 的名称/成员选择体验；打开、取消、草稿、loading 与错误保留在 PC。只有确认后的 `{display_name,user_ids}` 作为 `SurfaceActionRequested.input` 进入共享链。

**Rationale**: 组织树、焦点与弹窗生命周期都属于平台交互；`helix-im` 只需严格校验最终输入。

**Alternatives rejected**: 把候选成员树塞进 InteractionFrame 会扩大共享协议并让 core 知道 PC 组织 UI；继续 direct command 会产生第二条业务入口。

## R-003 HTTP、WS、sync 统一进入持久化屏障

**Decision**: 抽取共享 `ChannelCreatedCommit` 归一化路径，所有权威来源均只产生一个 `PersistAtomic`；匹配的 `PersistOk` 才能 patch dialog Frame。

**Rationale**: 当前 HTTP reply 和 `channel_created` 都会先发多个 storage effects，再在同一 tick patch Frame；`Effect` 向量不保证持久化先于可见性，已确认不满足目标合同。

**Alternatives rejected**: 依靠 Effect 列表顺序是隐式且不可证伪的；仅修 HTTP 会使 WS/恢复继续制造相同竞态。

## R-004 权威事实和 action completion 分离但可关联

**Decision**: HTTP reply 可以推进请求到等待/持久化状态，但不能单独宣告业务成功；创建者的最终 Frame mutation 与 action terminal receipt 使用原 request/causation identity。受邀成员收到自己的权威事实后，以自己的 user scope 形成 Frame，不复用创建者 action state。

**Rationale**: 建群是跨用户事实，只有创建者拥有发起 action；各用户的 visibility、unread 与挂载状态不同。

**Alternatives rejected**: 广播创建者 Frame 会泄漏 scope；只发 legacy `im:channel:created` 需要 PC 重建业务投影。

## R-005 幂等键使用权威 channel id，提交去重使用 request id

**Decision**: 提交阶段以 ActionInvocation/request id 防双击；权威事实阶段以 `channel_id` + user scope 幂等 upsert。重复 HTTP/WS/sync 事实允许重复进入归一化器，但不得形成第二行、第二成员关系或第二 terminal completion。

**Rationale**: 重试发生在权威 channel id 可用前后两个不同阶段，单一键无法正确覆盖两者。

**Alternatives rejected**: 名称+成员哈希会错误合并用户合法创建的两个同名群；只用 request id 无法去重断线恢复的权威事实。

## R-006 进入频道复用 GS04，不把发送纳入 G15a

**Decision**: 新行可见后，EnterChannel 继续走既有 GS04 summary + composer bundle；bundle 未到前禁用发送，timeline hydration 独立。

**Rationale**: G15a 的完成要求是可进入且具备消息输入能力，不要求自动拉完时间线或发送消息。

**Alternatives rejected**: 在建群完成 Frame 内嵌完整 timeline 会跨 surface join 并扩大原子批次；把“已发送消息”作为完成会混入另一个 UC。

## Resolved Discovery Questions

- **U-001 resolved**: 当前两条实时/HTTP 路径都没有严格等待 `PersistOk`；本计划以共享 `PersistAtomic` + correlation reply 修复。
- **U-002 resolved**: 受邀成员必须在各自 Helix session/user scope 处理同一权威事实并独立生成 Frame；现有 legacy raw emit 不能作为完成证据，需新增双用户 replay/真链证明。
