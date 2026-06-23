---
name: helix-loop-engine
description: 通用真 Go 联调 loop Engine —— 任意 sync-v2/IM 功能调试的「代码↔日志↔结果」三测自驱闭环。当用户要调试某功能为什么不工作（增量/已读/心跳/推送/断连/发送等真 Go 联调）、说"搭/跑 loop"、"loop engine"、要同时观察三层日志+代码+结果、要自动启动恢复+验证某功能时触发。不只针对 increment，是通用调试引擎。
---

# Helix loop Engine —— 真 Go 联调三测自驱闭环

> 一个**通用**调试引擎：给定一个功能/症状，自动「拉起环境 → 跑真 Go → 三层取证 → 定位失败层 → 修(helix)/报(Go) → 复验」循环，直到成功判据满足。
> 不绑定 increment——换功能只换 `FOCUS` 关键词。探针脚本 = `scripts/dev-loop.sh`。

## 启动即做（不要问，先拉起）

1. **环境自检/恢复**：`lsof -nP -iTCP:8065 -sTCP:LISTEN` 确认 Go 服务端在线；不在线 → 提示用户起 mattermost server（GoLand/`make run-server`）后再继续，**不要假装跑通**。Go 日志默认 `/Users/mac28/workspace/mmlog/mattermost.log`（变了设 `GOLOG=`）。
2. **明确本轮 FOCUS**：从用户症状提取关键词（增量=`increment|channel`、已读=`read|post_read`、心跳/断连=`ping|heartbeat|close`、发送=`post|send|echo`、同步=`sync|notify`）。

## 一圈循环（5 步，循环到成功判据满足）

```
跑 → 取证三层 → 定位失败层 → 修/报 → 复验  ↺
```

1. **跑**：`FOCUS='<关键词>' bash scripts/dev-loop.sh [秒数]`。它输出三层：① helix Effect/emit/警告 ② Go 日志 delta（FOCUS+错误/grpc/close）③ helix 收帧。
2. **取证三层**（缺一不可——这是"三测"的核心）：
   - **代码层①**：helix 吐了**预期的 Effect/emit** 吗？（如调增量该见 `increment HTTP triggered`；调已读该见对应 Http/emit）
   - **日志层②**：Go 收到请求了吗？worker/handler 走到哪步？有没有 `error/grpc/fail/panic`？（Go 日志是**对端真相**，比猜强）
   - **结果层③**：helix WS 实际**收到/没收到**哪些帧？emit 了哪些 `im:*`？
3. **定位失败层**（决策树）：
   - ① helix 没吐预期 Effect → **helix 代码 bug**（parser/handler/to_effect/module）→ 改 helix（白名单内）→ 复验。
   - ② Go 日志显示 Go 侧失败（grpc/store/panic/handler error）→ **读 Go 代码定位精确 file:line + 错误串**，给用户精确指针；**不改 Go**（ADR-008，helix 适配 Go 不改 server）。能力范围内的 helix 适配（缺头/缺字段/路径前缀）则改 helix。
   - ③ 代码①对、日志②对、结果③不符 → 帧解析/路由层（parser classify_frame / module dispatch / emit ACL）→ 改 helix → 复验。
4. **修/报**：
   - **可在 helix 修**（缺 header、URL 前缀、字段名、解析、handler 逻辑、timer 自清等）→ 改 + `cargo test -p helix-im` + `scripts/gate.sh` → commit（结构化 body，引用本轮三层证据）。
   - **Go 侧根因**（grpc 服务、Go 数据、Go handler bug）→ 写清「精确 Go file:line + 错误 + 为何 helix 改不了」给用户，标为待 Go 侧；helix 这边能并行做的别阻塞。
5. **复验**：同一 `FOCUS` 重跑 dev-loop → ③ 出现预期帧/emit + ② 无 error = 本圈闭合；否则带新证据进下一圈。

## 成功判据（每功能自定，开圈先声明）

进圈前先写死「怎样算修好」，例：
- 增量：③ 出现 `increment_channel`/`increment_channel_end` 帧 + helix emit `im:channel:increment` + ② worker 无 `grpc service fail`。
- 已读：③ 收 `post_read` + emit `im:post:read` + ② `CsesViewChannels` 无 error。
- 心跳：连接存活 > pingInterval（hello.pingInterval 实测 60s），② 无 `close from server: json_invalid`。

## 铁律

- **三层都要看**：只看 helix 日志会误判 Go 侧问题为 helix bug；只看 Go 会漏 helix 解析 bug。代码↔日志↔结果交叉验证才定得准。
- **Go 日志是真相源**：`/Users/mac28/workspace/mmlog/mattermost.log`，dev-loop 自动抓 delta。报 Go 侧问题必带 `caller`(file:line) + `error` 原文。
- **不改 Go server**（ADR-008）；helix 侧适配（头/字段/路径/解析）随便改，改完过 gate.sh。
- **不假装跑通**：Go 没起、帧没到、判据没满足 → 如实说卡在哪层，别糊。
- **沉淀**：同一根因栽 ≥ 3 次 → 即时落 `docs/harness/HX-C{NNN}` 坑卡（项目铁律）。

## 已沉淀的真 Go wire 事实（dev-loop 实测，复用免重查）

- 握手：连上 → 服务端单向下发 `hello{connectionId,pingInterval=60s}`；需 **WS 握手头 + HTTP 全局头双带 `cookieId`(=userId) + `companyId`**（缺任一 → 401/grpc fail）。
- HTTP api 前缀 `/api/cses`；WS 路由 `/api/v4/websocket`。
- 信封 `{v,action,data,broadcast,seq}`，action 顶层、seq 是信封序号（非 event_seq）。
- increment 是「HTTP 触发 → 后台 worker → WS 推 increment_channel×N+_end」非对称；worker 依赖 CsesGrpc `/Company/readIdNameWithCompany`（缺 company 数据 → `grpc service fail` 不推帧）。

## autonomous 驱动 host-cli 实战坑（2026-06-23 CI-4 真机联调沉淀，照做免重栽）

> dev-loop.sh 默认靠 TTY 喂 stdin；**autonomous / 后台 / 管道**场景 stdin 立即 EOF，必须显式保活。

1. **stdin 必须保活，否则 host-cli 秒退（最坑·误判成 helix bug 前先查这个）**：host-cli repl 读 stdin 到 EOF 即 `quit` 优雅退出。`run_in_background` / `| tail` / `< /dev/null` 下 stdin 立即 EOF → host-cli 启动 ~1ms 就退、WS 刚连上就断（症状三连：Go 报 `close 1006 EOF` + helix `tick_rx closed, draining` + ③ 收帧=0）。
   - 解：`{ sleep 4; echo "<命令>"; sleep 6; …; } | timeout 30 ./target/debug/helix-host-cli > LOG 2>&1`。首个 sleep 等 hello 握手盖 connectionId（否则首条命令在 connectionId=None 时发 → Go 401/无回推）；命令间 settle 等 WS 回推。纯连接/increment 观察用 `sleep 18 | host-cli`（只保活不发命令）。
2. **收帧靠 `HELIX_DUMP_FRAMES=1`** 才打印 `RAW WS inbound … frame={…action…}`；RUST_LOG **必带 `helix_driver_host=debug`**（WS 传输/握手在该 crate，缺则误以为没连上）。完整：`RUST_LOG='info,helix_im=debug,helix_driver_host=debug'`。
3. **DB 路径 quirk**：host-cli 用 `sqlite:{path}?mode=rwc`，真实文件名 **literally 含 `?mode=rwc`**（+`-wal`/`-shm`）。`ls /tmp/x.db` 找不到 → `ls /tmp/x.db*` 或直接 `sqlite3 "/tmp/x.db?mode=rwc" "…"`。表名**单数** `channel`/`message`/`channel_event_cursor`（cursor 列名是 `last_event_seq`）。
4. **查 DB 用 `sqlite3 -header -column` 防读错列**：`SELECT id,temporary_id` 与 `SELECT temporary_id,id` 列序相反，无表头极易把 echo 对账正确态误读成 swap bug（实测自己栽一次）。
5. **cursor 对账 phantom-aware**：`channel_event_cursor.last_event_seq`（如 554）> 该 channel message 行数（如 15）是**正常**——已读/编辑/删除/phantom 事件推 cursor 但不落 message。别拿 `cursor==msg_count` 当判据。

> **send echo 对账正解（CI-4④ 实测绿，复用免重查）**：乐观落库 `temporary_id=tmp`(PK) → Go 回 `post` 帧 `{id:server, temporaryId:tmp}` → parser（`temporaryId`→f.temporary_id / `id`→f.id，`parser.rs:37-40` 正确）→ `event_to_upsert_op` conflict on `temporary_id` 覆写 `id=server` → 终态 `temporary_id=tmp · id=server · send_status=sent`。
> **安全**：破坏性命令（create/close 建群解散）默认 SKIP，测试消息打清晰标记（如 `ci4-<phase>-<ts>`）；不改 Go（ADR-008）。

### 单连接 ping-pong 保活驱动（2026-06-23 沉淀·替代「每命令一次 host-cli」）

> **反模式**：`{ sleep;echo cmd; } | host-cli` 每条命令起一次 host-cli → 每次重连 + 重拉 increment(45ch)+sync。多命令时疯狂重连（实测 V6 一阶段 18 次重连、~14min thrashing）。
> **正解**：`scripts/host-cli-session.sh`——mkfifo 占住 stdin 写端 → host-cli **一次** connect+increment → 命令逐条喂同一连接 → 自带心跳 ping(8s) ping-pong 保活 → 全程零重连。更接近真实长连客户端，顺带覆盖心跳/长连稳定性。

```bash
# 用法：命令列表经 stdin（每行一条）
printf 'read CH\nnickname CH 新名\nsend CH hi\n' \
  | HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=<cid> bash scripts/host-cli-session.sh
```

**fifo 单连接铁律**（脚本已封装，手搓时照此序）：
```bash
mkfifo "$FIFO"
host-cli < "$FIFO" > LOG 2>&1 &   # 先起 reader（open 阻塞等 writer）
exec 3>"$FIFO"                     # 再占住写端解锁；占住期间 host-cli stdin 永不 EOF
sleep 7                            # 等唯一一次 connect+hello+increment+sync 落定
echo "read CH" >&3; sleep 5        # 命令逐条喂同一连接，settle 等 WS 回推
echo "nickname CH x" >&3; sleep 5
printf 'quit\n' >&3; exec 3>&-     # 关写端 → host-cli EOF 优雅退出
```

**验收**：脚本尾部打 `connect 次数`——**单连接铁律 == 1**（旧法 N 命令 = N 次）。实测 read+nickname+send×2 四命令：connect=1 / loadIncrementChannel=1 / syncChannels=45×1（旧法 4×45=180）/ 心跳 ping=1。
