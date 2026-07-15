# UC1-16 跨端纯渲染投影改造 SPEC

> 已废止：本文件只保留为 `RenderNode/RenderPatch` 迁移背景，不再是目标合同。新的权威设计见 `2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md`。

> 状态：Draft for implementation
> 日期：2026-07-15
> 范围：Helix `G-01..G-16` 主门禁及其 `a/b/c/offline` 变体
> 真源：`crates/helix-im/contracts/strict-matrix.v1.json`，contract epoch 4
> 非目标：本文件不修改 Rust、Angular、C++、QuickJS 运行时代码，不把 `declared` 误报为 Green。

## 1. 结论

PC Angular、Mobile QuickJS/C++ 与未来 Flutter 都必须退化为同一种薄壳：

```text
UI intent
  -> 透明 CommandPort / FFI
  -> Helix 领域状态机
  -> Helix Storage 持久化
  -> Helix render-ready RenderNode/v1
  -> 端侧非权威投影缓存
  -> UI 组件静态选择与渲染
```

端侧不得再拥有以下业务规则：

- 生成业务 `temporaryId`、`reqId`、revision 或消息身份；
- `temporaryId -> serverId` 对账、消息去重、合并、排序、淘汰和窗口组装；
- `sending/sent/failed`、撤回、加急、已读、回复树、成员角色和频道生命周期状态机；
- 从 `props`、原始 HTTP body、WS event 名或字段缺省推导展示语义；
- 自己计算 unread、mention、urgent、lastMessage、allowed actions；
- 用 HTTP 200、command accepted 或本地 optimistic mutation 充当业务完成。

端侧允许且必须保留的职责：

- 收集用户输入与设备 UI 事实，例如文本、选中文件、点击、滚动、视口可见性；
- 透明发送 intent，接收 RenderNode；
- 按 Helix 给出的 key、revision、mode、ordered keys 机械保存非权威 UI 缓存；
- 管理焦点、弹窗、滚动像素、选中态、动画和虚拟列表 DOM 生命周期；
- 按 `render_kind` 选择组件并绑定 props。

## 2. 当前版本基线

| 项目 | 当前事实 |
|---|---|
| Helix | `main@ce3e57e1ab5e50eaacfda33a3280eac7945a1f28` |
| cses-client | `feat/helix-reactor-im@b9f7dccff9278606b9f375b1eb0b7d8c4229697b` |
| cses-client Helix pin | 四个 Helix crate 均 pin `1e6f492dc0e00aa22f3ccf2e35da3310c430db89` |
| pin 对账 | `1e6f492` 是当前 Helix main 的祖先，但不等于 `ce3e57e`，当前分析必须记录版本差异 |
| mobile-qucik-c++ | `main@53666968857e2cf933c7e7fc9d26b76b33b2ab3c` |

实施前必须先 repin cses-client 的整个 Helix 依赖组并重跑合同检查；本 SPEC 只记录现状，不在脏工作树中擅自 repin。

## 3. 两种“保存”必须分开

### 3.1 Helix 持久化：权威本地事实

Helix Storage 保存可重启恢复的消息、频道、游标、成员、回执和发送状态。它是本地业务事实，不是 UI cache。

```text
Command / WS / HTTP reply
  -> domain transition
  -> Effect::Persist
  -> PersistOk
  -> Emit(RenderNode/v1)
```

关键约束：用户可见的业务终态投影原则上必须由已确认的持久化事实产生。发送乐观上屏也应先保存 `sending`，再 emit `message.row.v1`。当前 `handle_send_message` 在同一 step 依次吐 `Persist` 与 `Emit`，实施阶段应评估改成 `PersistOk -> Emit -> HTTP`，避免“屏幕有、重启无”。

### 3.2 端侧投影缓存：可丢弃视图

Angular `ProjectionStore` 或移动端 ViewModelStore 可以存在，但只能机械应用投影指令：

```json
{
  "schema": "RenderPatch/v1",
  "renderKind": "message.row.v1",
  "op": "upsert",
  "scopeKey": "channel:c1",
  "projectionKey": "tmp-123",
  "revision": 42,
  "orderedKeys": ["m-1", "tmp-123"],
  "node": { "...": "完整 render-ready props" }
}
```

缓存可以做 O(1) `Map.set`、按显式 `op` 删除、按显式 `orderedKeys` 覆盖顺序；不得从 event 名或 raw props 推导 patch。

## 4. 当前重复业务引擎

### 4.1 cses-client Angular

当前 `src/pages/message-v4/projection-store.service.ts` 仍包含第二套投影引擎：

- `messagesById`、`messageIdsByChannel`、`messageIdSetByChannel` 自己维护消息实体、顺序与去重；
- `temporaryToServerId`、`upsertMessageRow`、`linkMessageId` 自己完成临时 ID 对账；
- `visibleMessagesForChannel(..., 80)` 自己组装规范消息窗口；
- `upsertDialog` 自己做大量字段 fallback 与默认值裁决；
- `applyRepliesProjection` 再做 snapshot/append、revision、去重和 reply preview；
- `message-page-projection.presenter.ts` 再排序 dialog、再建回复索引；
- `message-read-reporting.ts` 根据 unread 与当前消息决定上报哪些已读；
- `ShellService` 生成 `temporaryId`/`reqId`、批量去重、选择历史 anchor、拼 reply/forward/schedule payload；
- `MessagePageComponent` 通过 `pendingReplySubmission + messageByTemporaryId` 再观察发送完成。

这些逻辑会迫使移动端复制同一规则，必须从 Angular 移除或降级为机械投影应用。

### 4.2 Mobile QuickJS/C++

当前移动端重复得更明显：

- `MobileImSdk::send` 在 C++ 生成 `temporaryId`、补默认 viewers/type、组装 core payload；
- `quickjs/bind_mobile_im.cpp::promoteServerId` 自己维护临时 ID 与服务端 ID 的 trace 映射；
- `MobileImSdk::applyEventProjection` 是 197 行 event switch，把 Helix event 再解释为 `channel_row/message_row/member_row/reply_row`；
- 它把任意 `im:post:*` 的非 sending 事件直接解释为 `sendStatus="received"`；
- 它自己从多套候选字段中猜 `channelId/msgId/text/type/userId`；
- 它自己解释 `dialogPatch`、同步 too_long、回复 IDs、成员字段；
- `ViewModelStore::applyPatchJson` 再按 `temporaryId/msgId/sendStatus` 删除重复行；
- QuickJS 暴露 `im.store.applyPatch`，允许 JS 绕过 Helix 直接制造业务投影。

这不是“移动 UI 适配”，而是第三套投影状态机，必须删除业务解释能力。

## 5. UC1-16 改造矩阵

下表按 Strict Matrix 的 `G-01..G-16` 编排；变体与主 Gate 同批治理。除 G-03 为 blocked/deferred 外，当前均只是 `declared/active`，不是发布 Green。

| Gate / UC | Helix 权威职责 | 当前端侧残留 | 目标端侧行为 |
|---|---|---|---|
| G-01 / UC-1.1 发文本 | 生成 ID；保存 sending；PendingSend/15s timer；HTTP；WS echo 对账；保存 sent；输出 message row | Angular/C++ 生成 temporaryId；两端自己做 tmp→server；页面观察 pending reply | `send({channelId,text,type})`；只消费 sending/received/failed 同 key 投影 |
| G-02 / UC-1.2 发文档 | 文档 command 规范化、wire body、上传/发送状态、完整 message row | Angular 把 attachment 当 props 拼装；C++ 补默认 type/viewers | UI 只提交文档选择结果/opaque upload handle；Helix 生成业务 props |
| G-03 / UC-1.3 媒体 | 上传计划、driver effect、对象键回报、消息状态与投影 | 两端理解 IMAGE/FILE 等 props 结构 | UI 只提供本地资源句柄、类型与用户说明；当前保持 blocked，不能假装完成 |
| G-03a / UC-1.4 重发 | 从 Storage 读取失败消息，校验可重试，重新进入 PendingSend，输出完整状态 | UI 传 messageId 并可能根据本地状态决定按钮 | UI 只在 `allowed_actions` 含 retry 时发 `retry` intent |
| G-04/G-04a / UC-1.5 撤回 | HTTP/离线 replay、message 终态、撤回文案/actor/capability | ProjectionStore 合并 revoke 字段与历史 text | UI 只覆盖同 `projectionKey` 的 revoked message row |
| G-05 / UC-1.7 转发 | 读取源消息、组装 Post、目标批次、每目标 accepted/delivery 状态 | Angular 去重/组装 source/target，维护批次展示结果 | UI 只提交用户选中的 ids/mode；消费 batch result 与目标消息投影 |
| G-06 / UC-1.8 emoji | 规范 emoji、HTTP、reaction 聚合与 action capabilities | 前端仍可能从 props/字符串生成 reactions | UI 发 reaction intent；直接渲染 `reactions/reactionCount` |
| G-07 / UC-1.9 加急 | 目标集合、确认态、required/confirmed counts、读回执与权限 | ProjectionStore 从 template/props/数组再次推 counts/readers | UI 发 target selection；渲染 urgent/read.receipt RenderNode |
| G-08 / UC-1.10 定时 | 时间合法性、temporaryId、schedule revision/status、持久化与投影 | Angular/C++ 组装 post 和 sendAt payload | UI 只传用户选择的时间和内容 intent；渲染 channel schedule row |
| G-09 / UC-1.10-cancel | 校验当前 schedule、取消终态、持久化与投影 | UI 依赖本地 channel 状态决定清理 | UI 发 cancel；收到 canceled projection 后关闭/刷新面板 |
| G-10/G-10a / UC-2.1/2.2 消息窗口 | SQLite query、limit、anchor、去重、稳定排序、窗口边界、older cursor | Angular `slice(-80)`、找 oldest anchor、merge/prepend；移动 ViewModelStore 自建 message table | UI 发 `open_channel/load_older` intent；Helix 返回 `message.window.v1` snapshot/append |
| G-11 / UC-2.3 定位 | 本地优先、远端补齐、目标上下文窗口与 highlight 事实 | UI 自己拿 postId 后再拼窗口/定位 | UI 发 locate intent；消费包含 ordered window + target key 的 projection |
| G-12 / UC-2.4 回复 | snapshot/append、revision、旧请求抑制、跨页去重保序、preview | Angular 再做 reply map/index/preview，移动端只存 replyId 行 | UI 只管理抽屉开关/滚动；直接渲染 `reply.tree.v1` |
| G-13 / UC-3.1 会话已读 | 根据 viewer/当前用户/游标决定 read range，写 channel/read state，输出 channel+receipt | Angular 根据 unread/messages 计算 candidate，维护 in-flight/completed sets | UI 只上报 `channel_visible/focus/viewport` 事实；Helix 决定是否及如何 mark read |
| G-14/G-14a / UC-3.2/3.3 单条已读/模板收到 | 作者豁免、幂等、receipt snapshot、templateReceived 终态 | `ShellService` 判断 author==self；loader 自己去重请求 | UI 只上报 message visible/confirm 点击；渲染 receipt/message row |
| G-15a/G-15 / UC-5.1/5.2 建群/话题 | 成员规范化、自己是否加入、频道/回复树持久化、created/members projection | Angular 维护 create baseline 并从 dialog 差集猜创建成功 | UI 发表单 intent；以 reqId 对应的 operation result + channel row 打开新群 |
| G-16/G-16b/G-16c / UC-5.3/5.4 关闭/退出/属性 | 权限、成员/频道终态、列表移除/禁用、设置 revision | 两端按 event 前缀猜 channel/member patch | UI 发 close/leave/settings intent；机械应用 channel/member RenderNode |

## 6. G-01 发送消息目标实现

### 6.1 Angular 目标代码

Angular 不再生成 `temporaryId`，也不等待 command response 变更消息状态：

```ts
async sendText(text: string): Promise<void> {
  await this.commandPort.invoke('im_send_message', {
    channel_id: this.activeChannelId(),
    text,
    type: 'TEXT'
  });
}

readonly messages = computed(() =>
  this.renderStore.select<MessageWindowNode>('message.window.v1', this.activeChannelId())?.items ?? []
);
```

Store 只接受显式 RenderPatch：

```ts
apply(patch: RenderPatch): void {
  const current = this.scopes.get(patch.scopeKey);
  if (current && patch.revision <= current.revision) return;
  this.scopes.set(patch.scopeKey, patch.node);
  this.version.update(value => value + 1);
}
```

这里的 revision 比较是 RenderPatch 协议规则，不是消息业务推导；如果 Helix直接给 snapshot，Angular 不需要做实体 merge。

### 6.2 Helix 目标代码形态

```rust
fn handle_send(intent: SendIntent, ids: &dyn IdSource, out: &mut EffectSink) {
    let temporary_id = ids.next_id();
    let message = PendingMessage::from_intent(intent, temporary_id);
    out.persist(message.to_sending_upsert(), Corr::SendingPersist);
}

fn on_sending_persisted(message: PendingMessage, out: &mut EffectSink) {
    out.emit(message.to_render_patch());
    out.http(message.to_create_post_request());
    out.schedule_timer(message.send_timeout());
}

fn on_post_echo(echo: PostEcho, out: &mut EffectSink) {
    let reconciled = reconcile_by_temporary_id(echo);
    out.persist(reconciled.to_sent_upsert(), Corr::EchoPersist);
}

fn on_echo_persisted(message: SentMessage, out: &mut EffectSink) {
    out.emit(message.to_render_patch());
}
```

同一条消息使用稳定 `projectionKey`；`temporaryId` 是 correlation，`serverId` 是远端命令身份，二者不再挤占 UI key。

### 6.3 Mobile QuickJS 目标代码

当前移动 API 已暴露 `im.send`，但不应再返回由 C++ 生成的业务 temporaryId，也不应让 JS 等 event 后自己对账。目标调用代码：

```js
// mobile-quick.js: UI intent
export async function sendText(channelId, text) {
  await im.commands.execute({
    name: "im_send_message",
    payload: { channel_id: channelId, text, type: "TEXT" }
  });
}

// 唯一数据入口：Helix RenderNode/v1
im.projections.subscribe("message.window.v1", projection => {
  mobileUi.renderMessageWindow({
    channelId: projection.scopeKey,
    items: projection.node.items,
    orderedKeys: projection.node.orderedKeys
  });
});
```

Flutter/QuickJS 可以保留 UI store，但只能保存收到的 node：

```js
const windows = new Map();

im.projections.subscribe("message.window.v1", projection => {
  windows.set(projection.scopeKey, projection.node);
  notifyUi(projection.scopeKey);
});

export function messagesFor(channelId) {
  return windows.get(`channel:${channelId}`)?.items ?? [];
}
```

目标 API 不再向 JS 暴露：

```text
im.store.applyPatch
promoteServerId
raw im:post:* -> message_row 解释器
按 event 名猜 sendStatus
按候选字段猜 id/text/type
```

### 6.4 Mobile C++ 目标代码

C++ 只做批次拷贝、ABI 解码与透明分发：

```cpp
void MobileImSdk::drainProjectionBatch() {
  for (auto&& envelope : bridge_.drainEvents()) {
    projection_sink_->publish(envelope); // 不解析 IM 字段，不制造 patch
  }
}
```

`ViewModelStore` 若因 Flutter 生命周期需要保留，应改为通用 `RenderNodeStore`：只认识 schema/op/scope/revision/node，不认识 message、channel、reply、unread、temporaryId。

## 7. 重启恢复

端侧 cache 丢失后不得自行恢复 pending map：

```text
App restart
  -> UI subscribe bus
  -> im_query_dialog_list
  -> im_query_messages_by_channel(channel, limit)
  -> Helix scan SQLite
  -> Helix 生成 dialog.list.v1 + message.window.v1 snapshot
  -> PC/Mobile 覆盖端侧 cache
  -> render
```

如果 SQLite 中仍是 sending，Helix 根据持久化发送合同决定恢复、超时或标失败；Angular/QuickJS 不重建 timer。

## 8. 改造影响面

### 8.1 Helix

主要目录：

- `crates/helix-im/src/send/**`：ID 生成、PersistOk 后投影、稳定 projection key；
- `crates/helix-im/src/query/render_ready/**`：统一 message window、locate、reply、dialog/member RenderNode；
- `crates/helix-im/src/channel/**`、`ws/handlers/**`：持久化后 emit 与同一 projection factory；
- `crates/helix-im/src/acl/**`：从事件碎片升级为 RenderPatch/RenderNode；
- `crates/helix-im/contracts/strict-matrix.v1.json` 与 schema：增加/冻结 window、operation result、projection key；
- `crates/helix-driver-native`、`helix-driver-ffi`：只透明转发，不做业务 normalization；
- 测试：每个 UC 证明 command -> Persist -> projection，破坏 Helix projection 后 PC/Mobile 同时 fail。

### 8.2 cses-client

主要目录：

- `src/pages/message-v4/projection-store.service.ts`：删除业务 merge/reconcile/window/reply 状态机；
- `projection.types.ts`、`view-model.types.ts`：改为生成的 RenderNode 类型；
- `message-page-projection.presenter.ts`：只保留纯展示格式，不排序/建领域关系；
- `message-render-props.ts`：不再从 props/成员表补业务字段；
- `message-read-reporting.ts`、receipt loader：降级为 viewport intent adapter；
- `shell.service.ts`：删除 ID、reqId、payload shaping、anchor 与批次业务规则；
- `message.component.ts`：删除 pending reply 对账、create baseline 猜测和业务完成观察；
- `src-tauri/src/features/helix_im/**`：保持 command/event 透明桥，不新增业务分支。

### 8.3 mobile-qucik-c++

主要目录：

- `cpp/MobileImSdk.cpp`：删除 `applyEventProjection`、业务 payload 组装与端侧 ID 生成；
- `cpp/ViewModelStore.*`：改为通用 RenderNodeStore，或由 Flutter UI store 取代；
- `quickjs/bind_mobile_im.cpp`：删除 `promoteServerId` 与 `im.store.applyPatch`，暴露 command/projection 两个通用面；
- `cpp/domain/MobileTypes.hpp`：业务 VM 类型改为生成的 RenderNode schema 或 opaque JSON；
- `demo/send_received_full_chain_demo.js`、smoke 与 gtest：从“等 event 并解释”改为“订阅 projection 并断言渲染 props”。

## 9. 迁移顺序

1. **P0 协议地基**：在 Strict Matrix 冻结 `RenderPatch/v1`、稳定 `projectionKey`、`message.window.v1`、operation result；生成 Rust/TS/JS/C++ 合同。
2. **P1 G-01 竖切**：发送文本先做到 PersistOk -> sending projection -> echo PersistOk -> sent projection；PC/Mobile 同一 fixture。
3. **P2 消息族**：G-02..G-07，删除两端 tmp/server、props、reaction、urgent、revoke 重算。
4. **P3 查询与回复**：G-10..G-12，Helix 输出完整窗口和 reply tree，删除 Angular/C++ 窗口与树组装。
5. **P4 已读族**：G-13/G-14，端侧只上报 viewport/focus intent，Helix 裁决 read range。
6. **P5 频道族**：G-08/G-09/G-15/G-16，删除 baseline/event-prefix 猜测，统一 operation result + channel/member projection。
7. **P6 硬切清理**：删除旧 ProjectionStore 业务 handler、Mobile `applyEventProjection`、`im.store.applyPatch` 与兼容 aliases；静态 gate 禁止回流。

## 10. 验收

### 10.1 静态边界

- Angular/C++/QuickJS 搜不到 `temporaryToServerId`、`promoteServerId`、业务 `makeTemporaryId`；
- 端侧不按 `im:post:*`、`im:channel:*` event 前缀推导 VM；
- 端侧不读取 raw `props` 推导 reactions/urgent/template/systemNotice；
- `im.store.applyPatch` 不再是公开 JS API；
- Tauri/FFI 不修改 command 名、payload、RenderNode。

### 10.2 行为

- 同一录制输入在 native 与 FFI 输出字节级等价 RenderNode 序列；
- PC 与 Mobile 使用同一 projection fixture，得到相同 key、顺序、状态和 allowed actions；
- 发送 HTTP 200 但无 WS echo 时，两端都不能显示 sent；
- echo 到达后同一行由 sending 收敛 sent，不重复、不闪烁；
- 重启后从 Helix SQLite query 重建相同窗口；
- out-of-order、duplicate、offline replay 只在 Helix 测一次规则，两端无对应业务测试副本。

### 10.3 复杂度

- Helix 单消息 apply：平均 O(1) keyed upsert；有序窗口 append O(1)，批量 snapshot O(n)；
- Angular/Mobile 应用完整 snapshot：O(n)，应用显式单行 patch：平均 O(1)；
- 禁止端侧在消息热路径中 `find/indexOf/sort` 组合成 O(n²)；
- 端侧虚拟滚动可以按 DOM viewport 做切片，但不能改变 Helix canonical ordered window。

## 11. 完成定义

某个 UC 只有同时满足以下条件，才算完成纯渲染改造：

```text
Helix direct command
+ persisted domain state
+ RenderNode/v1 完整字段
+ native/FFI 等价输出
+ Angular 无业务推导
+ Mobile QuickJS/C++ 无业务推导
+ PC DOM 与 Mobile UI 同一 correlation 收敛
+ Strict Matrix 八面证据
```

任何“Angular 页面看起来对”“QuickJS demo 收到 event”“HTTP 200”“ProjectionStore 有一行”都不能单独替代上述闭环。
