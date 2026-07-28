# MessageV4 Timeline V3 增量窗口 PRD

## 文档状态

- **状态**：已决策，待实施
- **日期**：2026-07-27
- **范围**：Helix、PC `cses-client`、Mobile QuickJS/C++ 的 Timeline Producer、Consumer、合同与验收
- **合同权威**：Helix
- **持久化事实源**：客户端真实 SQLite 消息库
- **Helix 副本来源**：cses-client `3000e97feb72039e4f9d30aa20a982f6e86c18c1`
  的 `docs/2026-07-27-message-v4-timeline-v3-prd.md`
- **关联规格**：[`specs/046-timeline-v3-delta-window/spec.md`](../specs/046-timeline-v3-delta-window/spec.md)

## 背景

当前 `InteractionFrame V2` 的 Timeline 更新只有完整 `replaceView` 语义。首次进入频道、发送消息、
收到新消息、撤回、回复或 Reaction 更新，均可能重新下发整个当前窗口。Angular 虽然可以依赖稳定
track key 复用部分 DOM，但仍需重新解码、校验并替换完整 Timeline ViewModel，且无法明确表达以下差异：

- 新消息只需要插入当前 Range 尾部；
- 消息终态、撤回、回复和 Reaction 只需要 Upsert 单条实体；
- 用户离开底部后，新消息应更新数据但不得抢走视觉 anchor；
- 上拉、下拉和定位加载需要扩展或安装 Range，而不是替换当前可见窗口；
- 后台频道的数据更新需要进入 SQLite，但不应强制恢复已淘汰的 Angular 消息体 Cache；
- revision 断档需要保留最后正确画面并从 SQLite 恢复，不能部分应用或由客户端猜测。

Timeline V3 将 Timeline 从“完整 View 替换”升级为“SQLite 权威、Range 驱动、窗口有界、原子增量”
合同，同时保持 Helix 独占消息排序、状态裁决和可渲染实体生成。

## 当前事实

- 当前 Frame 版本是全局 `InteractionFrame.schemaVersion=2`。
- 当前 Timeline mutation 是 `timelineSnapshot` 或携带 `replaceView` 的 `timelinePatch`。
- 当前 Angular Timeline 使用消息 ID 跟踪列表；本地乐观消息在服务端回声后可能从临时 ID 变化为
  服务端 ID，存在行身份变化风险。
- SQLite `message` 表以 `temporary_id` 为本地主键，`create_at` 为 `i64` 时间字段。
- 现有部分本地分页只使用 `create_at`，不能稳定区分同一时间戳内的多条消息。
- 当前 Helix attached Timeline 更新会发布完整 `replaceView`，PC 和 Mobile 均存在 V2 Fixture 与
  Consumer。

## 产品目标

1. 首次进入频道仍以完整 Snapshot 加载 SQLite 最新窗口；前端可声明 `pageSize`，缺省 20，
   当前合同上限 60。
2. 普通消息变化只发布有界 Timeline Delta，不再全量替换当前窗口。
3. 使用不可变的 `(createAt, temporaryId)` 作为消息实体身份和稳定排序键。
4. 发送消息从 `sending` 到 `sent/failed` 始终 Upsert 同一实体，不产生第二行，不重建原消息行。
5. 支持最新窗口、上拉更早消息、定位后向上/向下加载、返回最新和 Range 合并。
6. 用户处于底部时自动跟随；离开底部后固定视觉 anchor，只提示新消息。
7. Angular 维护有界 UI Cache，但不重新实现排序、状态裁决或消息业务语义。
8. SQLite 管控 `hasOlder`、`hasNewer`、最新消息水位和冷 Cache 重建事实。
9. Frame Delta 必须按 epoch/revision 原子应用；断档时冻结最后正确画面并恢复 Snapshot。
10. V3 平稳后物理删除 Timeline V2 生成、消费、Fixture 和兼容代码，不影响其他 V2 Surface。

## 非目标

- 不把 Dialog、UnreadSummary、ThreadDetail、ChannelSummary、Composer、MemberRoster、
  TopicList 或 ParticipantReceipts 同时改造成 Delta。
- 不删除其他 Surface 仍在使用的通用 `replaceView`。
- 不让 Angular 根据消息字段重新排序、推断撤回文案、计算发送终态或修复 revision。
- 不把 SQLite 当作尚未同步到本地的远端消息事实。SQLite 只权威表示当前已持久化的客户端事实。
- 不在本阶段执行百万消息、WS 风暴或长时间内存压力测试；只使用小预算确定性测试验证淘汰行为。
- 不引入通用 JSON Patch、数组下标 splice 或含糊的局部 replace 语义。

## 领域边界与依赖方向

| 层级 | 职责 | 禁止事项 |
|---|---|---|
| Helix Domain/Application | 消息身份、排序、可渲染实体、Range、revision、Snapshot/Delta、恢复编排 | 依赖 Angular DOM 或平台虚拟列表 |
| SQLite Repository | 持久化消息事实、复合游标查询、最新水位、`20+1` 探测 | 决定 UI 滚动行为 |
| Frame Producer | 将已持久化事实编码为版本化、原子 Timeline V3 mutation | 在持久化前发布成功状态 |
| SurfaceRuntime Cache | 机械校验和应用实体、Range、窗口操作，维护有界 Cache | 推导消息业务状态或自行排序 |
| Timeline Window Selector | 根据用户状态选择有界 RenderWindow，维护本地视觉 anchor | 改写权威 Range 顺序 |
| Angular Surface Adapter | 把 RenderWindow 映射为虚拟滚动列表和提示 | 全量扫描所有频道消息 |

采用的模式只对应真实变化点：

- **Repository**：SQLite 查询与持久化边界；
- **Transaction Coordinator**：Frame 暂存、校验和原子提交；
- **State**：`FollowLatest`、`Anchored`、`Locating`、`Hydrating`；
- **Strategy**：Cache Admission 与 Weighted SLRU 淘汰；
- **Adapter**：SurfaceRuntime Cache 到 Angular 虚拟滚动窗口；
- **Specification**：逐操作前置条件和 Range 不变量校验。

## 版本边界与切换策略

### 版本规则

- 不含 Timeline 的 `InteractionFrame V2` 可以继续运行，本次不要求其他 Surface 升级。
- 任何包含 `message.timeline` mutation 的 Frame 必须使用 `InteractionFrame V3`。
- `InteractionFrame V2` Consumer 一旦发现 Timeline mutation 必须 fail closed。
- V3 混合事务可以包含 Timeline V3 和未改变的其他 Surface V2 合同。
- V3 mutation 必须明确 `surfaceSchemaVersion`，避免 Frame 信封版本与 Surface 合同版本混淆。

示例：

```json
{
  "schemaVersion": 3,
  "transactionId": "tx-100",
  "mutations": [
    {
      "surface": "message.timeline",
      "surfaceSchemaVersion": 3,
      "mutation": {
        "kind": "timelineDelta"
      }
    },
    {
      "surface": "message.left-dialog-list",
      "surfaceSchemaVersion": 2,
      "mutation": {
        "kind": "dialogListPatch",
        "patch": {
          "kind": "replaceView"
        }
      }
    }
  ]
}
```

### 切换顺序

1. Helix、PC、Mobile 完成 Timeline V3 Producer、Consumer、Codec、Fixture 与回归能力。
2. 上线切换前全面关闭 Timeline V2 的运行时生成和消费入口。
3. 切换后所有 Timeline Frame 只走 V3；不存在能力协商、长期双发或运行时 V2 回退。
4. V3 验收期间保留不可达的 Timeline V2 代码作为版本回滚边界。
5. V3 真实链和回归测试平稳后，物理删除各仓 Timeline V2 代码与 Fixture。
6. 其他 Surface V2 合同、V2-only Frame 和通用 `replaceView` 保持不变，按各自计划独立演进。

## Timeline V3 核心合同

### EntityKey

每条消息的稳定身份和排序键只由 SQLite 事实组成：

```ts
interface EntityKey {
  readonly createAt: number;
  readonly temporaryId: string;
}
```

不变量：

- `createAt` 和 `temporaryId` 一经创建不得改变；
- `serverId` 是权威服务端字段，但不参与实体身份和列表排序；
- 所有本地消息必须拥有非空 `temporaryId`；
- Angular Map 可规范编码二元组作为字符串索引，但协议不得使用有碰撞风险的直接拼接；
- 发送对账继续通过 `temporaryId` 完成；
- Timeline 加载、定位、Range 和 DOM identity 使用 `EntityKey`。

### Timeline Snapshot

Snapshot 只用于：

- 普通登录或首次挂载频道；
- 冷 Cache 频道重新进入；
- epoch 变化或账户重建；
- revision 断档、非法 Delta 或事务失败后的恢复；
- 定位目标与当前所有缓存 Range 都无关系、且产品选择重建作用域时。

Snapshot 必须原子提供：

- 完整 render-ready entities；
- 一个或多个有序 Range；
- 当前活动窗口和 anchor；
- `timelineEpoch`、`timelineRevision`、`sqliteRevision`；
- Range 双向边界；
- 频道最新消息水位。

### Timeline Delta

V3 只允许以下明确操作：

```text
upsertMessage
installSegment
insertKeys
mergeRanges
setRangeBounds
activateWindow
```

#### `upsertMessage`

- 携带完整 render-ready Message ViewModel；
- Cache 命中时覆盖同一实体；
- Cache 未命中时可按 Admission 规则创建实体；
- sending、sent、failed、撤回、回复、Reaction、已读回执等均使用完整 Upsert；
- 撤回是 `revoke=true` 的更新，不是物理删除；
- 不负责排序或把实体放入 Range。

#### `installSegment`

- 安装一个新的、内部连续且已经排序的独立 Range；
- 用于定位未命中、返回最新但最新 Chunk 已淘汰、冷频道重建；
- 不覆盖其他 Range，也不隐式激活窗口。

#### `insertKeys`

- 向现有 Range 的 `start`、`end`、`beforeKey` 或 `afterKey` 插入已排序 keys；
- 必须携带期望边界，边界不匹配时整帧失败；
- 重复 key 必须幂等去重；
- 上拉、下拉、WS 新消息和迟到消息补洞均使用该操作；
- 不命令 Angular 改变滚动位置。

#### `mergeRanges`

- 只合并已经重叠或相邻、且排序关系可验证的 Range；
- 合并时按 `EntityKey` 去重；
- 不得为掩盖缺口而合并不连续 Range。

#### `setRangeBounds`

- 同时更新 `olderCursor/hasOlder` 和 `newerCursor/hasNewer`；
- boolean 与 cursor 必须一致；
- 所有值由 SQLite 查询结果产生，Angular 不得推断。

#### `activateWindow`

只表达显式导航：

```ts
interface ActivateWindow {
  readonly rangeId: string;
  readonly anchorKey: EntityKey;
  readonly reason: 'locate' | 'latest' | 'unread' | 'recovery';
  readonly align: 'start' | 'center' | 'end';
  readonly navigationToken: string;
}
```

禁止用于：

- 普通上拉或下拉；
- 新消息、发送状态、撤回、回复或 Reaction；
- Cache 预热、Range 合并或淘汰。

首屏由 Snapshot 的 `activeWindow` 初始化。缓存命中的定位或返回最新由本地 Window Selector 完成，
不要求 Helix 再发 `activateWindow`。像素偏移属于客户端本地状态，不进入 Helix 合同。

### 删除与移动

V3 不提供业务 `deleteMessage` 或 `moveKey`：

- 撤回通过 Upsert 表达；
- 发送失败保留原实体并 Upsert `failed` 状态；
- Cache 淘汰是本地动作，不是业务删除；
- `EntityKey` 不可变，因此没有合法的消息移动场景；
- 若权威排序事实无法保持，必须通过 Snapshot 恢复，不能发送 `moveKey` 修补。

## SQLite 分页与最新消息事实

### 请求

```ts
type TimelinePageRequest =
  | { readonly direction: 'latest'; readonly pageSize?: number }
  | { readonly direction: 'older'; readonly before: EntityKey; readonly pageSize?: number }
  | { readonly direction: 'newer'; readonly after: EntityKey; readonly pageSize?: number }
  | { readonly direction: 'locate'; readonly target: EntityKey; readonly pageSize?: number };
```

`pageSize` 缺失时取 20，合法范围为 `1..=60`。前端只能选择数量，不能提交排序、Range、
cursor、EntityKey 重写或服务端边界；Helix 对非法值 fail closed，不静默扩大为无界查询。

定位窗口的总实体数包含目标消息。双侧数据充足时：

```text
before = floor(pageSize / 2)
after  = pageSize - 1 - before
```

因此 `pageSize=20` 时为目标上方 10 条、下方 9 条；`pageSize=60` 时为上方 30 条、下方
29 条。目标保持为视觉居中 anchor；若一侧不足，未用额度自动补给另一侧，同时通过对应方向
Bounds 明确耗尽，不用空位或重复消息凑数。

### Range Bounds

```ts
interface RangeBounds {
  readonly olderCursor: EntityKey | null;
  readonly newerCursor: EntityKey | null;
  readonly hasOlder: boolean;
  readonly hasNewer: boolean;
}
```

### 频道水位

```ts
interface ChannelTimelineWatermark {
  readonly oldestPersistedKey: EntityKey | null;
  readonly latestPersistedKey: EntityKey | null;
  readonly sqliteRevision: number;
}
```

### 查询规则

- 最新、上拉、下拉和定位双侧均使用 `(create_at, temporary_id)` keyset pagination；
- 本地单方向查询使用 `pageSize + 1` lookahead；现有固定 20 条的远端接口按稳定锚点最多执行
  `ceil(pageSize/20)` 轮，并以服务端 `count` 或短批次判断耗尽。lookahead 和超出当前页的记录不进入
  RenderWindow；
- 上拉按复合键降序查询，返回前转换成 UI 正序；
- 下拉按复合键升序查询；
- 定位查询先把目标纳入窗口，再按 `before/after` 配额双向读取；一侧不足时把剩余额度补给另一侧，
  最终总数不超过 `pageSize`，双侧都充足时目标位于确定的中间索引；
- 目标索引是 `(channel_id, create_at, temporary_id)`，用于过滤和排序；
- 消息正文不进入巨大覆盖索引，按命中 row 回表读取；
- `hasOlder`、`hasNewer` 和水位必须在 SQLite 事务提交后产生并进入 Frame。

最新消息行为：

- 位于底部时，新消息持久化后插入最新 Range 尾部，窗口本地跟随，`hasNewer=false`；
- 离开底部时，水位前移、当前 Range 不动、`hasNewer=true`，UI 只提示新消息；
- 位于定位 Range 时，向下加载直至 `hasNewer=false`；
- 冷频道收到消息只更新 SQLite、水位和窗口新鲜度，不强制恢复消息体 Cache。

## Angular Cache 与虚拟滚动

### 运行时结构

每个频道拥有一个标准化 Cache：

```text
entitiesByKey
temporaryIdIndex
serverIdIndex
rangesById
latestWindowRef
pendingPins
activeWindow
```

- `entitiesByKey` 是消息实体主表；
- `temporaryIdIndex` 对已加载实体要求 100% 覆盖，用于发送对账；
- `serverIdIndex` 允许部分覆盖，用于服务端引用查找；
- Range 只保存有序 keys，不复制 Message ViewModel；
- RenderWindow 是 Range 上的有界选择，不拥有第二份业务实体；
- Surface attachment 只控制是否发布渲染，不控制 Cache 是否能接收符合 Admission 的更新。

### 滚动状态

```text
FollowLatest
Anchored
Locating
Hydrating
```

- `FollowLatest`：新消息到达时窗口滑向最新；
- `Anchored`：保存首个可见 `anchorKey + pixelOffset`，新数据不改变视觉位置；
- `Locating`：等待或选择目标 Range，只有当前 `navigationToken` 可激活；
- `Hydrating`：冷 Cache 或恢复期间保留最后正确画面，完成后原子切换。

### Cache 淘汰

采用 Chunk/Page 级 Weighted SLRU，不做逐消息 LRU。双预算至少覆盖实体数量和估算字节数。

必须 Pin：

1. 当前 RenderWindow 及上下各一个保护 Chunk；
2. 当前视觉 anchor 所在 Chunk；
3. sending/pending/等待对账的消息实体；
4. 正在执行分页或定位请求涉及的 Chunk。

Pin 必须使用引用计数和明确释放条件。淘汰只改变 Angular Cache，不删除 SQLite，不推进业务
revision。

### 各群最新窗口

所有频道保留轻量最新窗口引用，只有热频道保留完整消息实体：

```ts
interface ChannelLatestWindowRef {
  readonly channelId: string;
  readonly latestPersistedKey: EntityKey | null;
  readonly loadedFirstKey: EntityKey | null;
  readonly loadedLastKey: EntityKey | null;
  readonly loadedAtSqliteRevision: number | null;
  readonly residency: 'resident' | 'cold' | 'stale';
  readonly hasOlder: boolean;
}
```

- 淘汰后设置 `residency=cold`，清除 loaded key 范围，但保留 SQLite 水位；
- 冷频道收到新消息后更新水位并标记 `stale`，不回填完整 Chunk；
- 切换到 `cold/stale` 频道时重新从 SQLite 查询最新 `20+1`；
- `resident` 且 SQLite revision 一致时可以复用；
- 不能将旧窗口重新标记为最新。

## 并发、乱序与导航

- 每个加载或定位请求携带 `causationId`；
- 显式导航额外携带 `navigationToken`；
- 普通分页响应可以扩充 Cache，但永远不能激活窗口；
- 过期定位响应的数据可以进入 Cache，但其 `activateWindow` 是合同定义的条件性 no-op；
- `navigationToken` 不得绕过 epoch/revision 检查；
- Delta 仍必须满足 `baseRevision` 等于当前 revision，结果 revision 严格递增；
- 数据吸收和导航条件必须在同一暂存事务中验证后一次提交。

## 原子性与恢复

每个频道维护：

```text
timelineEpoch
timelineRevision
sqliteRevision
```

- `timelineEpoch` 表示账户重建或 Cache 世代边界；
- `timelineRevision` 表示连续 Frame Delta 顺序；
- `sqliteRevision` 表示窗口对应的持久化水位。

出现以下任一情况时 fail closed：

- epoch 不匹配；
- base revision 不匹配或 revision 断档；
- EntityKey、Range、边界或 cursor 不合法；
- 操作引用不存在且合同不允许创建的 Range/key；
- Frame 中任何其他 Surface mutation 非法；
- 暂存或提交失败。

恢复行为：

1. 保留最后正确画面，不清空、不跳底部；
2. 整个 Frame 不部分提交；
3. 标记相关频道 `resyncing`；
4. 合并重复恢复请求，只运行一个 Snapshot single-flight；
5. 优先围绕当前 anchor 从 SQLite 恢复；
6. anchor 不存在时降级最新 `20+1`；
7. Pending 实体和对账索引继续 Pin；
8. Snapshot 原子建立新 epoch/revision/sqliteRevision；
9. 丢弃 Snapshot revision 之前的迟到 Delta；
10. 其他频道不受影响，但同一 InteractionFrame 始终保持整帧原子性。

## 用户场景

### 首屏

登录并进入 A 群时，Helix 按前端 `pageSize` 从 SQLite 查询最新 `pageSize+1`，向 UI 提供至多
`pageSize` 条 Snapshot 和 `hasOlder`。未提供配置时使用 20；配置为 60 时同一算法提供至多 60 条。
这是普通流程中允许完整建立视图的场景。

### 底部发送

用户发送消息后，SQLite 先写入带稳定 `temporaryId` 的 sending 实体。Timeline V3 Upsert 并插入
该 `EntityKey`。服务端回声到达后使用同一 `temporaryId` Upsert 为 sent，并补充 `serverId`；
原消息行不得重复或重建。

### 离开底部收到消息

用户向上阅读历史消息时进入 `Anchored`。新消息进入 SQLite 和 Cache Range 尾部，水位更新，
当前 RenderWindow 与视觉 anchor 不动，只增加新消息提示。

### 上拉和下拉

上拉使用当前 Range 第一条 `EntityKey` 查询更早 `20+1`，通过 Upsert、`insertKeys(start)` 和
`setRangeBounds` 扩展。定位后的下拉使用末条 key 查询更新数据，直到 `hasNewer=false`。

### 快速定位

目标在 Cache 时，本地 Window Selector 直接定位。目标不在 Cache 时，Helix 按 `pageSize`
查询目标上下文；双侧充足时按 `floor(pageSize/2)` 条上方消息和剩余下方消息安装 Segment，目标
保持在确定中间索引并作为视觉 anchor。某侧不足时算法把剩余额度补给另一侧。随后原子发布 Upsert、
`installSegment` 和带当前 token 的 `activateWindow`，定位后仍可双向分页。

### 冷频道和 Pending 对账

用户在 B 群发送后切到 A 群。即使 B 群最新 Chunk 被淘汰，Pending 实体仍独立 Pin 并按
`temporaryId` 完成对账。非 Pending 的后台 B 群消息只更新 SQLite 和窗口水位。再次进入 B 群时
重新查询最新 `20+1`。

### 消息更新

- 可见消息：单实体 Upsert，只更新对应行；
- Cache 内不可见消息：Map 更新，不产生 DOM 工作；
- 已淘汰消息：只更新 SQLite，未来重新加载时反映最新状态；
- 撤回、回复和 Reaction 不改变 EntityKey 或 Range 位置。

## 性能与复杂度

- 单实体查找和 Upsert 目标复杂度为平均 `O(1)`；
- `temporaryId` 对账目标复杂度为平均 `O(1)`；
- 分页查询使用复合索引 keyset pagination，不使用 offset；
- 单次 Delta 成本与变更实体和 key 数量相关，不与整个历史消息总数相关；
- RenderWindow、单次 Frame、操作数量和 Cache 均必须有明确上限；
- 不允许 Angular 在每个 Frame 上扫描所有频道、所有 Range 或所有消息；
- Range 合并允许与参与合并的 key 数量线性相关，不得嵌套扫描实体表。

## 测试资产与 Fixture 同步迁移

Timeline V3 的交付范围不仅包含业务运行时代码。所有依赖 Timeline Frame 的生产者测试、canonical
JSON、同步工具、Mock、Consumer 合同测试、组件测试、真实链断言和静态 hard-cut Gate 都必须同步
迁移。只修改业务代码、但继续让测试期待 `timelinePatch + replaceView`，视为未完成。

### Gate 测试类与强制执行顺序

迁移按 Gate 纵切，不按“先把全部运行时代码写完、最后补测试”执行。每个测试类都必须先由 Helix
production `ImModule::handle -> Effect::Emit` 生成 Frame，再交给 PC 与 Mobile；consumer 不能反向
定义或修补业务 JSON。

第一批是**既有 Timeline 业务 Gate 迁移类**。它们覆盖 G-01/G-02/G-03a 的发送生命周期，
G-04/G-04a 的撤回，G-05 转发，G-06 Reaction，G-07 加急，G-10 首屏，G-10a 上拉，
G-11 定位，以及 G-14/G-14a 的消息行回执。每个 Gate 先把正向 Timeline Fixture 从
`timelinePatch + replaceView` 切到精确 V3 operation，并逐项证明 EntityKey 不变、普通更新不重建
完整窗口。

第二批是在全部既有 Gate producer Green 后新增的四组正式 Gate：

| Gate | 测试类 | Helix 正向操作合同 | PC/Mobile 消费合同 |
|---|---|---|---|
| `G-10b` | `LoadNewerGate` | 定位或中间 Range 之后执行 `upsertMessage* + insertKeys(end) + setRangeBounds`；普通分页不得 `activateWindow` | 下拉一页无重复/遗漏，耗尽时不重复请求，原 anchor 不被抢占 |
| `G-10c` | `FullWindowAppendEvictionGate` | 满窗口收到新消息只产 `upsertMessage + insertKeys(end)` 及必要边界更新；禁止 delete/move/replace/activate | RenderWindow 保持有界，头部 Cache 淘汰不改变业务 revision，稳定实体/行不重建 |
| `G-10d` | `AnchoredNewMessageGate` | 离底窗口收到新消息时更新实体、Range 与水位，但不发布 `activateWindow` | 当前视觉 anchor 与偏移不变，只增加新消息提示；点击提示后才返回最新 |
| `G-11a` | `UncachedLocateBidirectionalGate` | 未缓存定位发布 `upsertMessage* + installSegment + activateWindow`，随后 older/newer 分页分别只扩展 Range | 定位 token 只允许当前意图激活；定位后可双向分页，迟到响应只入 Cache 不抢窗口 |

四组 Gate 与 G-10/G-10a/G-11 基线都必须以 `pageSize=20` 和 `pageSize=60` 参数化运行。测试还要
覆盖非法 `0/61` fail closed、定位双侧充足时的确定中间索引，以及单侧不足时额度向另一侧回填。

每组 Gate 在三仓的固定资产名称如下；新增文件名不得复用旧 Gate 以免隐藏分母变化：

| Gate | Helix Scenario/Producer | Canonical 目录 | PC WDIO | Mobile GTest |
|---|---|---|---|---|
| `G-10b` | `support/timeline_formal_gate.rs` + `g10b_wdio_fixture_contract_test.rs` | `fixtures/g10b-wdio/` | `fixtures/g10b/` + `g10b-mock.e2e.ts` | `G10BFrameContractTest.cpp` |
| `G-10c` | `support/timeline_formal_gate.rs` + `g10c_wdio_fixture_contract_test.rs` | `fixtures/g10c-wdio/` | `fixtures/g10c/` + `g10c-mock.e2e.ts` | `G10CFrameContractTest.cpp` |
| `G-10d` | `support/timeline_formal_gate.rs` + `g10d_wdio_fixture_contract_test.rs` | `fixtures/g10d-wdio/` | `fixtures/g10d/` + `g10d-mock.e2e.ts` | `G10DFrameContractTest.cpp` |
| `G-11a` | `support/timeline_formal_gate.rs` + `g11a_wdio_fixture_contract_test.rs` | `fixtures/g11a-wdio/` | `fixtures/g11a/` + `g11a-mock.e2e.ts` | `G11AFrameContractTest.cpp` |

每个 canonical 目录至少包含 `provenance.json`、初始 Snapshot、目标 Delta 和必要的耗尽/失败负向
Frame；provenance inventory 必须逐文件登记 raw/semantic SHA-256、schemaVersion、
surfaceSchemaVersion、Surface inventory、producer revision 与参数化白名单。`.frame-v2.json`
只是一项历史文件名，不得用后缀推断合同版本；新增四组正向资产统一使用 `.frame-v3.json`。

四组正式 Gate 的最小正向 inventory 固定为：

| Gate | Canonical 文件 |
|---|---|
| `G-10b` | `latest.snapshot.frame-v3.json`、`initial-located.delta.frame-v3.json`、`newer-loaded.delta.frame-v3.json`、`newer-exhausted.delta.frame-v3.json`、`newer-late.delta.frame-v3.json` |
| `G-10c` | `full-window.snapshot.frame-v3.json`、`new-message-appended.delta.frame-v3.json` |
| `G-10d` | `anchored.snapshot.frame-v3.json`、`anchored-new-message.delta.frame-v3.json` |
| `G-11a` | `latest.snapshot.frame-v3.json`、`uncached-located.delta.frame-v3.json`、`located-older.delta.frame-v3.json`、`located-newer.delta.frame-v3.json`、`stale-navigation.delta.frame-v3.json` |

G-10c 的头部淘汰必须在 Consumer normalized window 断言，不得额外生成 Helix trim/delete Frame；
G-10d 的点击返回最新必须在 Window Selector/Action 断言，不得把点击预先编码进新消息 Delta。

强制顺序：

```text
既有 Helix Timeline Gate producer + JSON/provenance 全绿
-> G-10b/G-10c/G-10d/G-11a Helix producer + JSON/provenance 全绿
-> cses-client byte-exact sync + Codec/Runtime/组件/WDIO operation 断言
-> Mobile byte-exact sync + SurfaceRuntime/GTest/QuickJS operation 断言
-> 三仓 hard-cut 与 parity
```

任何前置批次失败，后续 consumer 可以保留已同步的只读诊断资产，但不得把该 Gate 标记完成或用端侧
手写 JSON 继续推进。

### 当前测试资产链路

| 所有者 | 当前资产 | V3 责任 |
|---|---|---|
| Helix | `crates/helix-im/tests/g*_wdio_fixture_contract_test.rs` 和 `tests/fixtures/*-wdio` | 从真实 Producer/Projector 路径生成 canonical V3 JSON、provenance 和 hash |
| PC Fixture 同步 | `scripts/message-wdio/sync-helix-fixtures.mjs`、`lib/fixture-hash.mjs` | 从固定 Helix revision 同步，不手改业务字段；校验 V2/V3 混合 inventory |
| PC Mock/合同测试 | `tests/e2e/wdio/fixtures/**`、fixture parity、SurfaceRuntime Codec/Service/DOM 测试 | 消费 canonical JSON，断言确切 Delta 操作、Cache/Range 结果和 DOM identity |
| Mobile Fixture 同步 | `scripts/sync-g*-helix-fixtures.mjs`、`scripts/lib/frame-v2-fixture-hash.mjs` | 升级为版本感知同步与 hash，继续保持 Helix 镜像 |
| Mobile Consumer 测试 | `tests/gtest/surface_runtime/G*FrameContractTest.cpp`、`SurfaceRuntimeTest.cpp` | 应用同一 canonical V3 JSON，断言实体、Range、窗口、拒绝和恢复结果 |
| Hard-cut Gate | PC/Mobile legacy hard-cut、source traceability、Fixture inventory 脚本 | 证明 Timeline V2 正向生成/消费/Fixture 归零，且非 Timeline V2 未被误删 |

首轮已知受影响 Gate inventory：

| Gate | 业务语义 | 必须迁移的 Timeline 断言 |
|---|---|---|
| G01/G02 | 文本/文档发送 | sending 插入、sent/failed 同实体 Upsert、WS 接收插入 |
| G03a | 失败与重试 | initial/retry sending、failed、retry sent 使用同一 EntityKey |
| G04/G04a | 在线/离线撤回 | recalled 只 Upsert `revoke=true`，恢复使用 Snapshot |
| G05 | 转发 | 目标频道新实体 Upsert 与 Range 插入 |
| G06 | Reaction | reacted 只 Upsert，恢复使用 Snapshot |
| G07 | 加急与确认 | urgent-active/confirmed 只 Upsert；Roster/Receipts 保持各自 Surface 合同 |
| G10 | Timeline 就绪、失败、降级与恢复 | Snapshot、最后正确画面和恢复合同 |
| G10a | 上拉与加载失败/竞争/耗尽 | Upsert、Range start 插入、Bounds 和失败不提交 |
| G11 | 消息定位 | 初始 Snapshot、目标 Segment 安装、条件激活和后续双向加载 |
| G14/G14a | 已读、模板等消息行变化 | 仅当 Timeline 行变化时 Upsert；其他 Surface mutation 保持原合同 |

G08/G09 的 ChannelSummary、G12 的 ThreadDetail、G15/G16 系列的非 Timeline 正向资产不因本次
改造自动升级。若某个 Fixture 是混合 Frame 且包含 Timeline，则只把 Frame 信封和 Timeline mutation
升级到 V3，其他 mutation 继续使用原 Surface 合同。

Helix Fixture 是业务合同唯一来源。PC 和 Mobile 的 JSON 必须是 Helix 产物的 byte-exact mirror，
并保留：

- `helixCommit`；
- `fixtureId` 和文件名；
- raw SHA-256 与 semantic SHA-256；
- `schemaVersion` 和各 mutation 的 `surfaceSchemaVersion`；
- Surface inventory；
- 允许 rebase 的运行时字段白名单；
- 生成该 Fixture 的业务前置条件和期望操作序列。

禁止 PC 或 Mobile 为了让测试通过而手工把 `replaceView` 改成 `upsertMessage`，也禁止同步脚本在复制
时重写业务 mutation。所有业务变化必须先在 Helix Producer 测试中生成，再同步给各 Consumer。

### Timeline Fixture 迁移规则

- 含 Timeline 的正向 Fixture 必须升级为 `.frame-v3.json` 和 Schema 3 provenance；
- 不含 Timeline 的 Dialog、Unread、Roster、Receipts 等 V2 Fixture 可以继续 `.frame-v2.json`；
- 同一个 JSON 同时包含 Timeline 与其他 Surface 时，整个 Frame 使用 V3，其他 Surface 保持自己的
  V2 mutation 合同；
- V2 Timeline Fixture 在切换期只能作为负向拒绝样本，不得继续驱动正向业务测试；
- V3 平稳并物理删除 Timeline V2 后，canonical 正向目录中不得再存在
  `timelinePatch + replaceView`；
- 若仍需防回归，使用最小负向协议样本证明 V2 Timeline 被拒绝，不能保留一套可运行的 V2 Timeline
  业务 Fixture；
- Fixture 文件不能只改版本号或文件名，mutation 内容和 Consumer 断言必须真实切换到 V3 语义。

### 业务操作与 Fixture 期望

| 业务场景 | Timeline V3 正向期望 |
|---|---|
| 首屏、冷频道重建、断档恢复 | `timelineSnapshot`，包含实体、Range、Bounds、Window 和水位 |
| 自己发送进入 sending | `upsertMessage + insertKeys(end)`，建立稳定 EntityKey |
| 自己发送变为 sent/failed | 仅 `upsertMessage` 同一 EntityKey，不生成第二条消息 |
| 接收方收到新消息 | `upsertMessage + insertKeys(end)`；Anchored 时不得激活窗口 |
| 撤回、离线撤回 | 仅 `upsertMessage(revoke=true)` |
| Reaction 新增/撤销 | 仅 `upsertMessage` |
| 加急、加急确认、模板或回执改变消息行 | 仅 `upsertMessage` |
| 转发产生目标频道新消息 | `upsertMessage + insertKeys` |
| 上拉更早消息 | 多个 `upsertMessage + insertKeys(start) + setRangeBounds` |
| 定位后下拉更新消息 | 多个 `upsertMessage + insertKeys(end) + setRangeBounds` |
| 定位 Cache 未命中 | `upsertMessage + installSegment + activateWindow` |
| 定位 Cache 命中 | 不需要新的 Frame；本地 Window Selector 行为测试 |
| Range 重叠或相邻 | `mergeRanges`，并断言稳定去重 |

上表描述的是给定 Fixture 前置状态下的确切操作合同。测试不得只检查最终文本存在，因为完整
`replaceView` 也可能得到相同最终 DOM，却违反增量合同。

### Helix Producer 测试要求

每个受影响 Gate 的 Producer 测试必须：

1. 从 production projector/handler 路径产生 Frame，禁止直接拼 JSON 作为正向权威；
2. 断言 Schema 3、Timeline Surface 版本、base/result revision 和 operation 顺序；
3. 断言 `EntityKey(createAt, temporaryId)`、Range 位置、Bounds 和水位；
4. 对 sending、sent、failed、撤回、Reaction 和加急分别证明目标是同一 EntityKey；
5. 对普通更新显式断言不存在 Timeline `replaceView`、`deleteMessage` 或 `moveKey`；
6. 导出 canonical JSON、provenance、raw hash 和 semantic hash；
7. 只允许 transaction、epoch、revision、causation 等声明过的运行时字段 rebase；
8. 不允许 rebase EntityKey、消息业务字段、Range 操作或 cursor。

### PC Consumer 与 Mock 测试要求

PC 测试必须同时更新：

- V3 strict Codec 的合法、非法、V2 Timeline 拒绝与混合 Frame 测试；
- SurfaceRuntime 原子应用、revision、Range、Cache、导航 token 和 Snapshot 恢复测试；
- `scripts/message-wdio` Fixture inventory、版本感知 semantic hash、provenance 与同步检查；
- WDIO Mock Fixture 文件名、loader 类型和每个 Gate 的操作级断言；
- 组件测试中的可见消息状态、anchor、新消息提示和稳定 DOM 行身份；
- Real Invoke/Multiremote 场景的 Frame 观测断言，不能只依赖 Mock 或最终 DOM。

Mock 只能参数化 provenance 明确允许的运行时字段，不得补写实体、改变操作类型或重新排序 keys。
测试必须分别证明 Producer Fixture、Consumer 状态和 DOM，不能互相替代。

### Mobile Consumer 测试要求

Mobile 必须：

- 从同一个 Helix revision 同步同一份 canonical JSON 和 provenance；
- 将 `frame-v2-fixture-hash` 等仅支持 V2 的工具升级为版本感知实现；
- 更新各 `G*FrameContractTest.cpp`、共享 `SurfaceRuntimeTest.cpp` 和 Fixture loader；
- 断言 V3 Delta 应用后的实体、Range、Bounds、活动窗口和恢复状态；
- 断言 V2 Timeline fail closed，但 Dialog-only V2 仍可消费；
- 更新 legacy hard-cut 与 source traceability 脚本，禁止继续把 Timeline V2 文件名和 mutation 当作
  正向必需资产；
- 与 PC 对相同 Fixture 输出等价的归一化状态，而不是只验证 JSON 可解析。

### 测试迁移完成定义

一个 Timeline 业务场景只有同时满足以下条件才算迁移完成：

```text
Helix production path 产生 V3
-> Helix Producer 测试锁定 operation
-> canonical JSON/provenance/hash 导出
-> PC byte-exact 同步并通过 Codec/Runtime/DOM/WDIO
-> Mobile byte-exact 同步并通过 GTest/Runtime
-> V2 Timeline 正向资产 hard-cut
```

任何一层仍断言 `timelinePatch + replaceView`，该场景保持未完成或 blocked，不得以其他层通过替代。

## 验收矩阵

以下场景必须以确定性功能测试覆盖，不以压力测试替代：

1. 登录后按 `pageSize+1` 查询，缺省 Snapshot 展示 20 条、配置 60 时展示至多 60 条；
2. 底部收到新消息并自动跟随；
3. 离开底部收到消息，anchor 不动，只提示新消息；
4. 上拉多页无重复、遗漏或视觉跳动；
5. 定位后按请求数量建立目标居中的窗口，再向上、向下分页并到达对应边界；
6. 定位命中 Cache 不访问 SQLite；
7. 定位未命中安装 Segment 后正确激活；
8. 返回最新在 resident 与 cold 两种状态下都正确；
9. sending 到 sent/failed 使用同一实体和 DOM 行；
10. 撤回、回复、Reaction 只 Upsert 对应消息；
11. 可见、Cache 内不可见和已淘汰消息更新分别符合预期；
12. B 群发送、切 A 群、B Chunk 淘汰后仍完成 Pending 对账；
13. 冷群后台消息不恢复完整 Angular Chunk；
14. 冷群重进时按当前 `pageSize+1` 从 SQLite 重新拉最新窗口；
15. Range 重叠去重、相邻合并、边界耗尽；
16. 分页与定位响应乱序时，过期导航不抢滚动位置；
17. revision 断档保留最后正确画面并恢复；
18. 非法操作或多 Surface Frame 任一失败时整帧不提交；
19. 使用极小 Cache 预算确定性触发 Chunk 淘汰、Pin 和释放；
20. V2 Frame 含 Timeline 时 PC/Mobile 均 fail closed；
21. V3 混合 Frame 中 Timeline V3 与其他 Surface V2 原子应用；
22. V3 平稳后删除 Timeline V2 不影响其他 Surface 的 V2 `replaceView`；
23. 发送 sending Fixture 是 `upsertMessage + insertKeys`，sent/failed Fixture 只 Upsert 同一
    EntityKey；
24. 撤回、Reaction、加急和消息行回执 Fixture 只 Upsert 目标消息，不出现 Timeline
    `replaceView`；
25. 上拉、下拉、定位和 Range 合并 Fixture 分别携带正确 Range 操作与边界；
26. Helix、PC、Mobile 的 Timeline V3 JSON raw hash、semantic hash、provenance 和 Surface inventory
    一致；
27. PC Mock 和 Mobile GTest 不修改 Helix 业务字段，只参数化 provenance 允许的运行时字段；
28. Timeline V2 正向 Fixture、测试期望和生成入口归零，Dialog 等非 Timeline V2 Fixture 继续通过。

## 成功标准

- 除首次加载和恢复外，普通消息变化不产生 Timeline 全量 `replaceView`。
- 所有发送生命周期测试均只存在一个稳定 EntityKey 和一个消息 DOM 行。
- 上拉、下拉、定位、返回最新和后台新消息场景均不发生非用户意图的滚动跳转。
- 所有 `hasOlder/hasNewer/latestPersistedKey` 均可追溯到已提交 SQLite 查询事实。
- 断档和非法 Delta 测试中不存在部分提交或端侧业务推断。
- 小预算淘汰测试中 Cache 保持有界，冷频道可从 SQLite 正确恢复。
- PC 与 Mobile 对同一 canonical V3 Fixture 得到等价的归一化 Timeline 状态。
- 每个受影响 Gate 的 Helix Producer 测试、canonical JSON、PC Mock/Consumer 测试和 Mobile
  GTest 必须作为一个迁移单元同步完成。
- 发送、撤回、Reaction、加急、加载与定位的正向 Fixture 均断言具体 V3 operation，不能只断言最终
  DOM 或字段值。
- Timeline V2 运行时入口在 V3 切换前全部关闭，V3 稳定后代码被物理删除。

## 风险与约束

| 风险 | 控制 |
|---|---|
| 全局 Frame 版本升级误伤其他 Surface | V2 非 Timeline Frame 保留；只有含 Timeline 的 Frame 必须 V3 |
| EntityKey 编码碰撞 | 协议使用结构化二元组，客户端使用统一规范编码 |
| 同毫秒消息分页遗漏 | SQL 和 Range 顺序均使用 `(createAt, temporaryId)` |
| 前端把窗口数量放大为无界扫描 | `pageSize` 缺省 20、上限 60，非法值 fail closed；查询与跨边界输出始终有界 |
| 偶数窗口无法数学上单条绝对居中 | 固定 `before=floor(N/2)`、`after=N-1-before`，再由本地视觉 anchor 居中 |
| Cache 淘汰破坏发送对账 | Pending 实体独立 Pin，`temporaryIdIndex` 保持覆盖 |
| 迟到定位响应抢走视图 | `navigationToken` 条件激活 |
| 恢复期间 UI 闪白或跳底部 | 冻结最后正确画面，anchor 优先 Snapshot |
| 双轨长期存在 | Timeline V2 只保留短期版本回滚代码，V3 平稳后物理删除 |
| Fixture 被误当真实链 | 合同、SQLite、Frame、DOM 和跨端证据分别验收 |

## 发布门槛

Timeline V3 切换必须同时满足：

- Helix Producer 和 canonical Fixture 已冻结；
- PC、Mobile Consumer 与严格 Codec 已通过同一 Fixture；
- V2 Timeline 生成和消费入口已有机器可验证的 fail-closed Gate；
- SQLite 复合游标、真实持久化顺序，以及 `pageSize=20/60` 的 `pageSize+1` 结果已验证；
- 首屏、发送、WS 新消息、上拉、下拉、定位、恢复和淘汰功能测试通过；
- Helix 产出的 canonical V3 JSON 已通过 provenance/hash 校验并 byte-exact 同步到 PC 和 Mobile；
- 所有受影响 PC Mock、SurfaceRuntime、组件、WDIO、Mobile GTest 与 hard-cut 脚本已迁移到 V3
  operation 断言；
- 混合 Frame 的原子性回归通过；
- 回滚只能通过版本回滚，不允许运行时降级到 Timeline V2；
- V3 稳定验收完成后有独立任务删除 Timeline V2 代码。
