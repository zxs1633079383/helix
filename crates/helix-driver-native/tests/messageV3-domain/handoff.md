# MessageV3 Phase1-4 真实 Gate Handoff

## 交接对象

- **仓库**：Helix
- **worktree**：`/System/Volumes/Data/workspace/worktrees/messagev3-phase1-real-chain/helix`
- **分支**：`codex/messagev3-phase1-real-chain`
- **Go 服务**：`/System/Volumes/Data/workspace/golang/cses-im-server-MessageV3-helix`，运行端点 `http://127.0.0.1:8066/api/cses`、`ws://127.0.0.1:8066/api/v4/websocket`
- **Angular 真源**：`/System/Volumes/Data/workspace/angular/visual/cses-client`
- **提交**：`a6ab29fc`（生产事件契约修复）、`e7492a1a`（Phase1-4 真实 Gate harness）、`15e9fdcf`（缺口清单与交接记录）
- **推送状态**：未 push

## 本轮完成事项

1. 固定 84 个 Gate 的 Rust target，按 `PhaseN/Domain/Gate/main.rs` 拆分并在 `message_v3_domain.rs` 注册：Phase1=33、Phase2=20、Phase3=15、Phase4=16。
2. 将所有 Gate 改为默认执行；真实前置缺失时写 `BLOCKED_PRECONDITION`、`NOT_VERIFIED` 或 `NOT_APPLICABLE`，不使用 `#[ignore]` 隐藏缺口。
3. 共享 native harness 接入真实 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink` 和 Helix engine loop，覆盖 Tick/Effect、Go HTTP、Go WS、SQLite 和 `im:__bus__` 证据。
4. 修复 G06b 的 author/reader Helix session 竞态，验证真实 `im_post_read`、read event 和 SQLite row。
5. 修复 G09c 的 authenticated actor identity，并把成员昵称事件统一为 `im:channel:member-nickname`；Phase3 15/15 通过。
6. 为 Phase4 Pin、Bookmark、Announcement、Search、Todo 补真实 probe；没有 target command 或终态 event 时写出跨仓根因。
7. 清理文档统计口径：README、SESSION 和本 handoff 均区分 Cargo harness 结果与业务 Gate evidence 结果。

## 本轮媒体测试类修复（不启动 WDIO）

1. 根据 Angular `messageFileUpload.service.ts` 与 `@ccc/core-common` `FileService` 的现行实现，纠正媒体 owner：文件字节走 Java `fileDrive`/OSS；Go 8066 只消费完成后的稳定媒体引用并执行 `posts/create`，旧 Rust `/media/upload/prepare` 探针不再作为 Angular 真链依据。
2. `Phase1/Upload/G01c,d,e,g` 改为稳定 `picture/attachment` 引用的真实 Go post 闭环，分别覆盖多图、文件、AUDIO/VIDEO（duration/尺寸）和 TEMPLATE/IMAGE；每个测试仍检查 `Tick::Command → sending → Go HTTP → WS post → received → SQLite sent → im:__bus__`。
3. `Phase1/Upload/G03c` 改为 `postIds + channelIds` 的真实图片 preview 转发，核对 source/target 的 `bucket/id/uri` 稳定引用一致，并证明转发不重复 prepare/PUT。
4. `Phase1/Upload/G02d` 改为 Helix planner 的精确 `67,108,865` bytes 负向 Gate，验证在 Go/OSS/SQLite/业务 WS 之前拒绝，写入 local-contract PASS。
5. 本轮没有启动 WDIO；因此媒体 Gate 的 Java/fileDrive/OSS 字节级上传仍明确写为 `BLOCKED_PRECONDITION`，不会把 stable-reference fixture 误报为完整媒体 PASS。

## 验证结果

### Focused real-chain harness

| 命令范围 | 结果 |
| --- | --- |
| `phase1_` | 33 passed, 0 failed |
| `phase2_` | 20 passed, 0 failed |
| `phase3_` | 15 passed, 0 failed |
| `phase4_` | 16 passed, 0 failed |

这里的 passed 表示测试函数执行并写证据，不等于所有业务 Gate PASS。

### Rust 工程验证

- `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web`：通过。
- `cargo test -p helix-core`：22 个可运行测试通过。
- `cargo test -p helix-im --lib`：313 passed, 0 failed。
- `cargo build --target wasm32-unknown-unknown -p helix-driver-web`：通过。
- `cargo fmt --all -- --check`：通过。
- `git diff --check`：通过。
- Go `/api/cses/health`：HTTP 200；仅作为服务可达前置，不作为业务 Gate 证据。

## 当前业务证据账

按每个注册 Gate 选取最新 `evidence.json`：

```text
Phase1: 21 PASS / 6 BLOCKED_PRECONDITION / 5 NOT_VERIFIED / 1 NOT_APPLICABLE
Phase2:  9 PASS / 7 BLOCKED_PRECONDITION /  0 NOT_VERIFIED / 4 NOT_APPLICABLE
Phase3: 15 PASS / 0 BLOCKED_PRECONDITION /  0 NOT_VERIFIED / 0 NOT_APPLICABLE
Phase4:  6 PASS / 10 BLOCKED_PRECONDITION / 0 NOT_VERIFIED / 0 NOT_APPLICABLE
总计:   51 PASS / 23 BLOCKED_PRECONDITION / 5 NOT_VERIFIED / 5 NOT_APPLICABLE
```

证据目录：`artifacts/messagev3-phase1-4-real-chain/`。运行证据未加入 Git，且不包含账号、cookie、密码、正文或完整 URL。两个历史中间 `MV3-G05j-create` 目录已可恢复地移至 `/tmp/messagev3-stale-g05j-create-20260811`，避免把非注册 Gate 计入 84 Gate 汇总。

## 待办与责任边界

### Go/后端侧

- **Media**：Angular 当前字节上传 owner 是 Java `fileDrive`/OSS，不是 Go `/media/upload/prepare`；需提供真实 Java prepare、OSS PUT/complete 与 sender/receiver read-back，同 Rust stable-reference 子链对账后再晋级 G01c/d/e/g、G03c。
- **Post producer/readback**：`internal/app/cses_post_write.go:205` 调用 `CreateCsesPost`；`internal/app/cses_post_create.go:57-107` 负责 producer 投递。当前部分 post admission 后无法从 `/posts/get` 读回，阻塞 Phase2 Timeline 和 Phase4 Announcement read。
- **Pin**：真实 route 位于 `internal/app/cses_channel_manager_pin.go:195-331`；需要检查 post pin WS producer、event payload 和 consumer 落库，使 Helix 能收到 `im:post:updated`。
- **Search**：当前 Go 只有 `/search/post`、`/search/user`、`/search/channel`、`/search/do`；Angular 目标 global/in-channel/grouped command 尚未形成三方注册合同。
- **Todo**：`/posts/queryTodoList` 的 explicit read 已可返回，但自动 `im:todo:updated` 未形成；需要明确 Go payload/WS 或 Helix projection owner。

### Angular/native 侧

- G01h/G01i/G01j 需要保持 gap 边界或提供真实 producer/capability/schema；隐藏入口不能计作 PASS。
- G02e/G02d/G05l/G06c/G06e/G11f/G13e 是本地 owner 或 zero-write/lifecycle Gate，需要真实宿主或 UI evidence，不应强行接 Go mutation。
- 若补充 Angular/WDIO/Native 证据，必须通过 `gateId`、`corr/req_id/temporaryId` 与本 Rust evidence 对账；DOM、静态扫描和 HTTP 200 不能独立晋级。

## 推荐下一轮执行顺序

1. 补 Angular Java `fileDrive`/OSS 的真实 media fixture 与同轮证据，再跑 Phase1 G01c/d/e/g/G03c；不要把 Go media repository 约束当作当前 Angular 上传链路。
2. 修/观测 Go post producer 到远端读库的屏障，再跑 Phase2 Timeline；同屏障成功后跑 Phase4 G07d。
3. 修 Pin WS→Helix projection→SQLite，再跑 G05f/G05g。
4. 明确 Search/Todo 三方合同后跑 G14a-c/G15a-d。
5. 补本地 owner Gate 的 Tauri/Angular evidence，保留 N/A 的合同边界。
6. 严格执行 Phase1→Phase2→Phase3→Phase4，并重新生成 84 Gate 汇总；任何一 Gate 缺终态都保留非 PASS。

## 重跑入口

```bash
cargo test -p helix-driver-native --test message_v3_domain phase1_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase2_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase3_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_ -- --nocapture --test-threads=1
```

不需要用户设置环境变量；Go 仍必须先运行在 `localhost:8066`，测试自动发现 Angular credentials。
