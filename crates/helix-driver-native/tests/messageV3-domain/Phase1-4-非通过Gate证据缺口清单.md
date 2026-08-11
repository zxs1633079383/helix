# MessageV3 Phase1-4 非通过 Gate 证据缺口清单

## 结论

本清单基于 `codex/messagev3-phase1-real-chain` worktree 中每个已注册 Gate 的最新 `evidence.json` 汇总。84 个 Gate 的分母固定为：Phase1=33、Phase2=20、Phase3=15、Phase4=16。

| Phase | Gate 数 | PASS | BLOCKED_PRECONDITION | NOT_VERIFIED | NOT_APPLICABLE |
| --- | ---: | ---: | ---: | ---: | ---: |
| Phase1 | 33 | 21 | 6 | 5 | 1 |
| Phase2 | 20 | 9 | 7 | 0 | 4 |
| Phase3 | 15 | 15 | 0 | 0 | 0 |
| Phase4 | 16 | 6 | 10 | 0 | 0 |
| 合计 | 84 | **51** | **23** | **5** | **5** |

`cargo test` 中的 `passed` 只表示 Gate harness 执行完并写出证据；本表的 `PASS` 才表示业务链已经满足 HTTP、Go WS、Helix projection、SQLite 持久化和事件总线等合同。当前缺口不是通过增加等待时间或改成静态断言解决，必须补齐下面列出的真实证据。

证据根目录：

```text
artifacts/messagev3-phase1-4-real-chain/<run-id>/<Phase>/<Domain>/<Gate>/evidence.json
```

## 状态含义与晋级规则

### `BLOCKED_PRECONDITION`

测试已经触达真实链，但在 Go producer/consumer、远端读回、WS fan-out、Helix projection 或跨仓合同处失败。媒体 Gate 还要求 Angular 的 Java/fileDrive/OSS 字节上传证据；Rust 测试只从稳定引用开始时，必须保留该外部前置阻塞。清障后必须重新跑同一 Gate，并在同一轮证据中看到失败层之后的全部终态；不能把 HTTP 200 或 health 200 当成 PASS。

### `NOT_VERIFIED`

当前没有被批准的真实 fixture、生产 producer、宿主负向夹具或 retry 起始行，因此测试只记录缺口。清障需要先提供真实输入和合同，再补齐 HTTP/WS/SQLite/Emit 证据；禁止用合成 WS、fake server 或静态 replay 替代。

### `NOT_APPLICABLE`

该 Gate 的 owner 是 Angular/native 本地生命周期或零写入边界，没有独立 Go mutation。它不能靠 Go real-chain 晋级；要么保持 N/A 并提供对应的本地/宿主证据，要么补充明确的 Helix local assertion 后重新定义终态。

## Phase1：缺口 12 个

### BLOCKED：`Post/MV3-G03a`

当前 `posts/createPosts` admission 没有在目标频道产生预期的真实 WS `post`，证据将第一失败层定位为 Go `createPosts` producer/WS fan-out 或目标频道成员关系。

清障所需的完整证据：

1. Go `posts/createPosts` 返回 `SUCCESS`，并返回每一个 source post 对应的 target `postId`/`temporaryId`。
2. 同一认证连接收到目标频道真实 WS `post`，字段中的 `postId`、`temporaryId`、`channelId` 与 HTTP 接受事实一致。
3. Helix 收到 `im:post:received`，写入 target SQLite `send_status=sent`，并由 `NativeEventSink`/`im:__bus__` 发出终态。
4. 多 post 转发时数量、顺序、source→target 对应关系一致；不能只证明 HTTP accepted。

证据文件：`Phase1/Post/MV3-G03a/evidence.json`。Angular 的真实转发合同在 [Phase1 Handoff](/System/Volumes/Data/workspace/angular/visual/cses-client/tests/e2e/wdio/phase1/Handoff.md:161)。

### BLOCKED：Upload 媒体引用与转发

Rust 测试已经使用当前 Angular 链路产出的稳定媒体引用，真实打到 Go `posts/create`/`posts/createPosts`，并断言 Go WS `post`、Helix `im:post:received`、SQLite `sending→sent` 与 `NativeEventSink/im:__bus__`。这些 Gate 仍不能标成完整 PASS，因为本轮没有启动 Angular/WDIO，也没有 Java `fileDrive`/OSS 的字节级 prepare、PUT、complete 同轮证据；旧的“Go `media.type NOT NULL`”不是当前 Angular 媒体链的阻塞根因。

| Gate | 当前已验证 | 完整 Gate 仍缺的证据 |
| --- | --- | --- |
| `Upload/MV3-G01c` | 两个 `picture` 稳定引用、`posts/create`、WS、Helix projection、SQLite 与 emit。 | 两张真实图片由 Angular `FileService` → Java/fileDrive → OSS 完成 prepare/progress/PUT/complete，且 sender/receiver read-back 同轮一致。 |
| `Upload/MV3-G01d` | `attachment/phase1-file.txt` 稳定引用和 Go 消息闭环。 | 真实 `phase1-file.txt` bytes 的 Java/fileDrive/OSS 上传与文件名/大小 authority read-back。 |
| `Upload/MV3-G01e` | AUDIO（duration=2、无尺寸）与 VIDEO（duration=2、320x180）分别完成 Go/WS/Helix/SQLite/emit。 | 两个真实媒体 fixture 的 Host 元数据、Java/fileDrive/OSS 上传平面与双 persona projection。 |
| `Upload/MV3-G01g` | Canvas 生成图的 `TEMPLATE/IMAGE` 稳定引用和 Go 消息闭环。 | `im_stage_generated_media`、真实正尺寸 bytes、Java/fileDrive/OSS 上传和模板 sender/receiver read-back。 |
| `Upload/MV3-G03c` | `postIds + channelIds` 图片 preview 转发，source/target 稳定引用一致，prepare/PUT 计数为 0。 | source 图片必须先由真实 Angular Java/fileDrive/OSS 链产生，并提供 target read-back 的跨仓证据。 |

`Upload/MV3-G02d` 已从缺口移出：Rust 直接在 Helix planner 边界拒绝精确 `67,108,865` bytes，并以 `effect_count=0`、Go/OSS/SQLite/业务 WS 计数为 0 写入 PASS。该 PASS 是 Helix local contract，不替代 Angular/Tauri UI 宿主的视觉证据。

### NOT_VERIFIED：Draft、Post

| Gate | 当前事实 | 清除所需证据 |
| --- | --- | --- |
| `Draft/MV3-G02e` | draft 按合同属于本地 `HostStorage`，没有 Go 8066 mutation。 | 若保持本地 owner：补 `im_save_draft → HostStorage row → 切换频道 → im_query_draft/read-back`，按 `(accountId, channelId)` 隔离，证明 HTTP/WS/domain event 为 0；若要求 real Go，则必须先定义并实现 Go draft API/WS 合同。 |
| `Post/MV3-G01h` | TASK_CARD 是外部 inbound，当前没有批准的真实 Go fixture/WS producer。 | 认证的生产 Java/task producer 通过 Go WS 向当前 channel 发布 TASK_CARD；Helix 收到对应 action，完成 projection、SQLite/read-back 和 consumer 对账。Angular 当前明确要求 `BLOCKED_EXTERNAL_TASK_CARD_PRODUCER_UNAVAILABLE`，见 [Phase1 Handoff](/System/Volumes/Data/workspace/angular/visual/cses-client/tests/e2e/wdio/phase1/Handoff.md:202)。 |
| `Post/MV3-G01i` | VOTE create 入口隐藏，只有 contract，没有真实 create producer。 | 若要做 real PASS：补 capability/schema、create command、Go mutation、WS authority、Helix projection 和 durable read-back；若产品决定保持隐藏入口：补 zero-effect/gap evidence，证明无 Angular intent、无 Native mutation、无 HTTP/WS 写入，不能把入口隐藏算成功。 |
| `Post/MV3-G01j` | AVERAGE_SCORE create 与 G01i 相同，当前没有真实 fixture/terminal projection。 | 同 G01i：补真实 producer 链，或记录隐藏入口的 zero-effect/gap evidence；DOM 隐藏本身不是 real-chain PASS。见 [Phase1 Handoff](/System/Volumes/Data/workspace/angular/visual/cses-client/tests/e2e/wdio/phase1/Handoff.md:203)。 |
| `Post/MV3-G02c` | 没有被批准的 durable failed row 和真实 retry 起始 fixture。 | 先获得同一 `temporaryId` 的 `send_status=failed` 行及失败原因；再触发真实 `im_retry_send`，证明 HTTP admission、同 temporaryId、真实 WS authority `post`、canonical server id、`failed→sent`、旧 timer 取消和单 bubble 终态。Angular retry 合同见 [real-send-fault.e2e.ts](/System/Volumes/Data/workspace/angular/visual/cses-client/tests/e2e/wdio/phase1/specs/real-send-fault.e2e.ts:78)。 |

### NOT_APPLICABLE：`Post/MV3-G05l`

这是 toolbar zero-write：业务上没有 Go mutation。当前缺少的不是 Go 证据，而是 Helix/native 本地负向断言。补齐后应证明：positive-control post 之外，精确时间窗内没有 `im_*` 业务 command、storage write、HTTP、非心跳 WS、projection 或 `im:__bus__` 业务 Emit；只允许虚拟列表的只读 refresh。Angular 合同见 [Phase1 Handoff](/System/Volumes/Data/workspace/angular/visual/cses-client/tests/e2e/wdio/phase1/Handoff.md:201)。

## Phase2：缺口 11 个

### NOT_APPLICABLE：本地生命周期/派生状态

| Gate | Owner 与当前判定 | 需要补的证据 |
| --- | --- | --- |
| `Read/MV3-G06c` | Angular active-dialog/viewer-local 生命周期，没有 Helix command、Go HTTP、WS 或 SQLite mutation。 | 若继续按现有 owner：补真实 Tauri/Angular 生命周期证据（激活、切换、取消、恢复）并维持 N/A；不能用 Go read 代替。 |
| `Read/MV3-G06e` | unread aggregate 由 Angular 从 Helix `channel.unreadCount` 派生，没有独立后端 mutation。 | 补 channel projection → Angular 派生角标的真实宿主/UI read-back；不能为了 real-chain 强行增加 Go mutation。 |
| `Sync/MV3-G13e` | 冷启动/断线/重连恢复属于 native/Angular 调度生命周期，没有独立 Go mutation。 | 补真实断开、重连、恢复调度、cursor/本地持久化和 UI 可见终态；保留与 Go HTTP/WS 业务 mutation 的边界。 |
| `Timeline/MV3-G11f` | viewer window/scroll 是本地视觉生命周期，没有 Go HTTP/WS/storage mutation。 | 补 native/browser viewport、anchor、scroll window 的真实生命周期证据；不能用 timeline authority 成功冒充。 |

### BLOCKED：Timeline authority/远端持久化 7 个

这些 Gate 的共同前置是：真实 Go producer 写入远端 post store 后，使用同一 `postId/channelId/temporaryId` 能从 Go read route 读回；然后 Helix 才能证明 `im:read:result`、cache Persist 和 SQLite durable row。当前有的请求返回错误，有的返回 `SUCCESS` 但 `posts=[]`。

| Gate | 当前第一失败层 | 清除所需证据 |
| --- | --- | --- |
| `Timeline/MV3-G11a` | Go producer/consumer 或 remote post persistence；latest post create 后 `/posts/get` 读不到同一 post。 | `posts/create` 的 post 经过 producer/consumer 落远端库；`posts/get` 返回同一 server id；Helix latest command 收到正确 `im:read:result`，写 cache/SQLite，并读回完整 row。 |
| `Timeline/MV3-G11b` | `/api/cses/posts/getPostsAfterIndex` older authority 返回错误。 | Go older route 成功返回 anchor 前窗口、req_id 与 page metadata；Helix 发出匹配 req_id 的 `im:read:result`，持久化 page 并确认 `hasMore/anchor`。 |
| `Timeline/MV3-G11c` | `/api/cses/posts/getPostsAfterIndex` newer authority 返回错误。 | Go newer route 成功返回 anchor 后窗口、连续游标和尾部状态；Helix req_id、Persist、SQLite read-back 全部对应同一 channel/window。 |
| `Timeline/MV3-G11d` | `/api/cses/posts/postContext` locate authority 返回错误。 | Go `postContext` 返回精确 target post 和上下文；Helix `im_locate_message` 只消费该 req_id，持久化 target row，不混入别的 channel/post。 |
| `Timeline/MV3-G11e` | 与 G11a 相同，重复 latest 读仍读不到 Go authority。 | 先证明第一次 latest post 已在 Go 读库可见；第二次读取复用同一 authority contract，不引入 queryCache 旁路，并保留同一 durable row。 |
| `Timeline/MV3-G11g` | Go `/posts/get` 返回 SUCCESS-shaped 但没有 seeded post。 | remote store 必须含 seed `postId`；精确 `im_get_posts` 返回该 id；Helix `im:read:result`、SQLite `send_status=sent` 和 temporary→server 对账一致。 |
| `Timeline/MV3-G11h` | initial `/getPostsAfterIndex` 结果不含 seeded message。 | Go initial page 返回目标 post 及合法 page size/cursor；Helix `im_get_posts_after_index` 产生匹配 req_id、连续 page、Persist 和本地 read-back。 |

相关 Go producer 代码：`/System/Volumes/Data/workspace/golang/cses-im-server-MessageV3-helix/internal/app/cses_post_write.go:205` 调用 `CreateCsesPost`；`internal/app/cses_post_create.go:57-107` 负责异步/producer 投递。timeline route 注册和处理位于 `internal/api/post/register.go:44-51`、`internal/api/post/handlers_read.go:287-401`。

## Phase3：无非通过 Gate

Phase3 当前 15 个 Gate 全部 PASS，没有 BLOCKED、NOT_VERIFIED 或 N/A。G09c 的真实身份处理和 canonical `im:channel:member-nickname` 事件名已经修复并复验；后续重跑仍需保留 actor/observer 双身份、Go member mutation、Helix event 和 SQLite member row 的同轮证据。

## Phase4：缺口 10 个

### BLOCKED：Announcement、Pin

| Gate | 当前第一失败层 | 清除所需证据 |
| --- | --- | --- |
| `Announcement/MV3-G07d` | 公告保存后的异步 `CreateCsesPost`/post read producer 没有让真实公告 post 在 `/posts/get` 可见；当前等待 20 秒仍为 `SUCCESS` + 空 data。 | 先用公告实际持久化 `postId` 从 `/posts/get` 读到同一 post；再执行 Phase1 `im_post_read` + `im_post_read_list`，收到 canonical `im:post:updated`/read projection，确认 SQLite post/read bits 和 observer 分流。`/post/announcement/read` 不是本 Gate 的 authority。证据文件中保留了该合同说明。 |
| `Pin/MV3-G05f` | `im_set_message_top` 已到 Go pin route，但没有 Helix `im:post:updated` terminal projection。 | 同一已 sent post：Go `/api/cses/channel/add/postPinned` 成功；Go WS post update 到达；Helix 发 `im:post:updated`；SQLite `pinned=true`；再次 query/read-back 仍为 true。Go handler 见 `internal/app/cses_channel_manager_pin.go:195-331`，Helix handler 见 `crates/helix-im/src/ws/handlers/post_pin.rs:1-93`。 |
| `Pin/MV3-G05g` | 与 G05f 相同，取消置顶没有 terminal projection。 | Go `/api/cses/channel/remove/postPinned` 成功；WS update 到达；Helix `im:post:updated`；SQLite `pinned=false`；load/read-back 确认取消置顶且不残留旧状态。 |

### BLOCKED：Search 合同未闭合

| Gate | 当前事实 | 清除所需证据 |
| --- | --- | --- |
| `Search/MV3-G14a` | Angular 目标为 `im_search_global / im_search_messages_grouped`，Helix 当前只有 `im_search_do` 等现有命令；Go 只有 `/search/do` 等现有路由，替代探测不能晋级目标 Gate。 | Angular command、Helix outbound registry/capability、Go route/permission/DTO 三方注册一致；真实 marker keyword 请求返回 `im:read:result`，结果包含 seeded post/user/message，req_id/correlation 对齐。 |
| `Search/MV3-G14b` | 目标 `im_search_in_channel` 未注册；当前 `im_search_post` 探测返回 access denied，不能代替目标合同。 | 补 in-channel command、channel scope wire、Go `/search/post` 或新 canonical route 的权限和结果；证明只返回目标 channel，Helix event/read-back 字段完整。 |
| `Search/MV3-G14c` | 目标 `im_search_messages_grouped` 未注册；当前 `im_search_do` 只证明旧命令存在。 | 补 grouped command/response schema 和 Go authority；真实结果按 message group 聚合，保留 marker、channel/post identity、req_id，并完成 Helix `im:read:result`。 |

### BLOCKED：Todo 自动投影未闭合

| Gate | 当前事实 | 清除所需证据 |
| --- | --- | --- |
| `Todo/MV3-G15a` | `im_query_todo_list` 的明确查询返回 `im:read:result`，但没有自动 `im:todo:updated`。 | urgent post 真实创建后，自动 todo event 必须包含 seeded post、`type=urgent`、`canDelete=false`、状态字段，并与 Go `/posts/queryTodoList`、Helix correlation、SQLite/read-back 一致。 |
| `Todo/MV3-G15b` | urgent confirm mutation 可产生 post update，但 todo automatic projection 仍缺失。 | 在 confirm 后收到同一 post 的 todo update；证明确认状态、recipient/confirmed 字段、`canDelete=false` 与 read result 一致，不能只看 `im:post:updated`。 |
| `Todo/MV3-G15c` | mention 查询可读到 channel/post，但缺自动 todo event。 | 真实 mention post 后收到 `type=mention`、`canDelete=true`、目标 user/状态字段的 `im:todo:updated`，并与 Go query/read-back 对账。 |
| `Todo/MV3-G15d` | 与 G15c 相同，当前只有显式 read result，没有自动投影。 | 补同一 mention post 的自动 todo event、列表状态字段、req/correlation 和 SQLite/read-back；不能把 explicit `im_query_todo_list` 成功算成 Gate PASS。 |

## 建议清障顺序

1. **Angular media plane**：用真实 Java `fileDrive`/OSS fixture 补同轮 bytes、prepare/PUT/complete 与 read-back，再把 G01c/d/e/g、G03c 从“稳定引用子链”晋级完整 Gate；不要把 Go `/media/upload/prepare` 当成 Angular 当前上传入口。
2. **Go post producer/read authority**：确认 `CreateCsesPost(s)` 的 consumer、远端 post store 和 `/posts/get` 读回，再重跑 Phase2 G11a/e/g/h、G11b/c/d，并复用同一屏障验证 G07d。
3. **Pin projection**：沿 Go pin handler → WS → `post_pin` → SQLite 更新逐跳加证据，重跑 G05f/g。
4. **Search/Todo 合同**：先完成 Angular command、Helix registry/capability、Go route/DTO 的三方注册，再跑 G14a/b/c、G15a-d。
5. **本地 owner Gate**：为 G02e、G02d、G05l、G06c、G06e、G11f、G13e 补 native/Angular 宿主证据；不为了统计好看而强行接 Go。
6. 每次修复都按 **修复 → 单 Gate focused → 对应 Phase → Phase1→Phase4 串行复验 → 84 Gate 汇总** 执行；状态只有在终态证据齐全时才从 BLOCKED/NOT_VERIFIED 晋级 PASS。

## 重跑命令

Go 必须运行在 `localhost:8066`；不要求用户设置环境变量，测试自动读取 Angular credentials：

```bash
cargo test -p helix-driver-native --test message_v3_domain phase1_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase2_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase3_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_ -- --nocapture --test-threads=1
```

### 证据边界

- Go health `200 OK` 只代表服务可达。
- HTTP `status=SUCCESS` 只代表 admission/read response，不代表远端持久化、WS fan-out 或 Helix terminal。
- fake server、静态 replay、仅 Cargo 通过、仅 Angular DOM、仅 SQLite 本地行均不能替代真实 Gate 证据。
- artifacts 保持未跟踪，避免把账号、cookie、正文或运行态凭据提交进 Git。
