# MessageV3 Phase1-4 非通过 Gate 证据缺口清单

## 当前结论

本清单按当前 worktree 中每个注册 Gate 的最新 `evidence.json` 汇总，更新时间为 2026-08-11。84 个 Gate 的分母固定为 Phase1=33、Phase2=20、Phase3=15、Phase4=16。

| Phase | Gate 数 | PASS | BLOCKED_PRECONDITION | NOT_VERIFIED | NOT_APPLICABLE |
| --- | ---: | ---: | ---: | ---: | ---: |
| Phase1 | 33 | 22 | 5 | 5 | 1 |
| Phase2 | 20 | 16 | 0 | 0 | 4 |
| Phase3 | 15 | 15 | 0 | 0 | 0 |
| Phase4 | 16 | 16 | 0 | 0 | 0 |
| 合计 | 84 | **69** | **5** | **5** | **5** |

证据根目录：

```text
artifacts/messagev3-phase1-4-real-chain/<run-id>/<Phase>/<Domain>/<Gate>/evidence.json
```

`cargo test` 的成功只表示 harness 执行并写出了证据；只有最新 `evidence.json` 的 `status=PASS` 才计入业务 PASS。HTTP 200、Go health、静态注册、fake server、合成 WS、单独 SQLite 行和测试函数退出码都不能替代完整 Gate 终态。

## 已完成的两批目标

### 第一批：运行态修复后复验的 9 个

以下 Gate 均已获得同轮 Go HTTP、Go WS、Helix projection、SQLite read-back 和 NativeEventSink/`im:__bus__` 证据：

```text
Phase1: G03a
Phase2: G11a、G11b、G11c、G11d、G11e、G11g、G11h
Phase4: G07d
```

Timeline 测试不再把固定 `createAt=0` 当作 authority anchor，而是从 Go create response 提取真实 `createAt`；G03a 使用真实 `posts/createPosts` target read-back；G07d 的 announcement seed 带上 Go 所需 `teamId`。

### 第二批：代码合同修复后复验的 9 个

以下 Gate 均为 PASS：

```text
Pin:    G05f、G05g
Search: G14a、G14b、G14c
Todo:   G15a、G15b、G15c、G15d
```

Pin 的真实运行态根因是 Go `channel/add/postPinned` 触发 PostgreSQL `42P10`：旧库没有 `post_pinned(channel_id,post_id)` 联合唯一约束，虽然 Ent schema 已声明唯一索引。当前 Go store 使用不指定 conflict target 的兼容写法，使旧库不再被 `42P10` 阻断；同时新增了需批准后手工执行的 `sql/006_post_pinned_unique.up.sql`，正式补回部分唯一索引。没有在本轮直接改真实数据库，也没有把 HTTP `status=SUCCESS` 单独算作 PASS。G05f/g 的最新证据分别证明置顶与取消置顶的 Go route、WS `post_pin`、`im:post:updated`、SQLite `props.pinned` 和 read-back。

## 仍未晋级的 15 个 Gate

### BLOCKED_PRECONDITION：5 个媒体 Gate

这 5 个 Gate 已经证明“稳定媒体引用消费”子链：`Tick::Command → sending → Go posts/create → Go WS post → im:post:received → SQLite sent → emit`。当前缺的是 Angular 真实字节上传平面，而不是 Go `media/upload/prepare`：现行 Angular owner 是 Java `fileDrive`/OSS。由于本轮按要求不启动 WDIO，不能把稳定引用 fixture 晋级成完整媒体 Gate。

| Gate | 已有证据 | 仍缺证据 |
| --- | --- | --- |
| `Phase1/Upload/G01c` | 两个 picture 稳定引用消费、Go/WS/Helix/SQLite/emit | 两张真实图片经 Angular `FileService → Java fileDrive → OSS` 的 prepare/PUT/complete、大小/URI authority read-back |
| `Phase1/Upload/G01d` | attachment 稳定引用消息闭环 | `phase1-file.txt` 的真实 bytes 上传及文件名/大小 authority read-back |
| `Phase1/Upload/G01e` | AUDIO 与 VIDEO 的 duration/尺寸元数据消费闭环 | 两个真实 fixture 的 Host 元数据与 Java/fileDrive/OSS 字节平面、双 persona read-back |
| `Phase1/Upload/G01g` | Canvas `TEMPLATE/IMAGE` 稳定引用消费闭环 | `im_stage_generated_media`、正尺寸 bytes、Java/fileDrive/OSS 上传和 sender/receiver read-back |
| `Phase1/Upload/G03c` | `postIds + channelIds` 图片 preview 转发，且不重复 prepare/PUT | source 图片先经过 Angular Java/fileDrive/OSS，并提供 source/target 跨仓 read-back |

晋级条件：同一轮证据必须包含真实 bytes 的 prepare、PUT、complete、Go 消费、WS fan-out、Helix projection、SQLite durable row 和 emit；不得只补 Rust stable-reference fixture。

### NOT_VERIFIED：5 个

| Gate | 当前事实 | 晋级所需证据 |
| --- | --- | --- |
| `Phase1/Post/G01h` | TASK_CARD 是外部 inbound，当前没有批准的真实 Go/WS producer fixture | 认证生产 producer 发布 TASK_CARD；Helix 收到 canonical action，完成 projection、SQLite/read-back 和 emit |
| `Phase1/Post/G01i` | VOTE create 入口当前只有合同，未形成真实 producer/终态 projection | 三方 capability/schema、Go mutation、WS authority、Helix projection 和 durable read-back；或明确记录隐藏入口 zero-effect gap |
| `Phase1/Post/G01j` | AVERAGE_SCORE 与 G01i 相同，暂无真实 fixture | 同 G01i；DOM/入口隐藏本身不是 real-chain PASS |
| `Phase1/Post/G02c` | 没有批准的 durable `send_status=failed` 起始行和 retry fixture | 同一 temporaryId 的 failed row、真实 `im_retry_send`、HTTP/WS authority、`failed→sent`、timer 取消和单终态 bubble |
| `Phase1/Draft/G02e` | draft 按当前合同属于本地 HostStorage，没有 Go 8066 mutation | 本地 `im_save_draft → HostStorage → 切换频道 → query/read-back` 及 HTTP/WS/domain-event 为 0；若改为 Go owner，先定义 Go 合同 |

### NOT_APPLICABLE：5 个

| Gate | owner 与边界 | 需要的证据（不通过 Go 强行晋级） |
| --- | --- | --- |
| `Phase1/Post/G05l` | toolbar zero-write，没有 Go mutation | Helix/native 负向断言：无业务 command、storage write、HTTP、业务 WS、projection 或 emit |
| `Phase2/Read/G06c` | Angular active-dialog/viewer-local 生命周期 | Tauri/Angular 激活、切换、取消、恢复的宿主证据 |
| `Phase2/Read/G06e` | Angular 从 `channel.unreadCount` 派生 unread aggregate | channel projection 到 UI 角标的宿主/read-back 证据 |
| `Phase2/Timeline/G11f` | viewer window/scroll 本地视觉生命周期 | viewport、anchor、scroll window 的 native/browser 生命周期证据 |
| `Phase2/Sync/G13e` | native/Angular 冷启动、断线、重连调度生命周期 | 断开、恢复、cursor、本地持久化和 UI 终态证据 |

这些 Gate 的 N/A 是 owner 边界，不代表测试漏跑；只有业务合同改变时才应改为 Go mutation Gate。

## 证据和复验命令

当前分支与 worktree：

```text
/System/Volumes/Data/workspace/worktrees/messagev3-phase1-real-chain/helix
codex/messagev3-phase1-real-chain
```

Go 服务端点：`http://127.0.0.1:8066/api/cses`、`ws://127.0.0.1:8066/api/v4/websocket`。测试自动读取 Angular 现有凭据，不要求用户配置环境变量。

已完成的重点复验命令：

```bash
cargo test -p helix-driver-native --test message_v3_domain phase1_post_g03a::mv3_g03a_real_chain -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase2_timeline_g11 -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_announcement_g07d::mv3_g07d_real_chain -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_pin_g05f::mv3_g05f_real_chain -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_pin_g05g::mv3_g05g_real_chain -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_search -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_todo -- --nocapture --test-threads=1
```

Go 修复后的聚焦验证：

```bash
go test ./internal/store/sqlstore -run 'TestCsesPost_Pinned$' -count=1
go test ./internal/app -run 'Test.*(Pinned|Pin)' -count=1
go build ./...
go vet ./...
```

重新计算账面时，按 Gate 选择最新修改时间的 `evidence.json`，不要按运行目录数量相加；这样可避免历史重跑目录或临时 Gate 污染 84 的分母。
