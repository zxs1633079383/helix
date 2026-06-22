# Golden fixtures — 真 Go 帧回放素材

这里存的是 **真 Go 服务端实际下发的 WS 帧原文**（不是手写猜测的帧）。
配套测试 `tests/golden_replay_test.rs` 加载这些 fixture → 喂 `ImModule(Tick::Inbound)`
→ 断言 Effect / 落库 / cursor / emit 序列。

## 为什么要 golden fixtures（这条「链」要解决什么）

本会话最大坑：parser 曾按**猜测**写 action 名 `"posted"`，真帧其实是 `"post"`。
根因——**测试帧是手写猜的**，和现网 wire 漂移了 CI 也测不出。

golden fixtures 把「测试用帧」从「手写猜测」升级为「现网真抓」：

- fixture = 真 Go 下发原文（带来源头注释，可追溯抓帧日期/形态）；
- 测试 = 回放断言；
- 任何 parser 字段路径 / action 名漂移 → 回放断言立刻红。

「链」= 让后人能**持续补真帧**：每遇一个新帧形态（新 action / 新字段路径），
抓真帧 → 存 fixture → 加回放断言，护栏随现网演进。

## 文件格式约定

每个 `*.json` fixture：

- **第 1..N 行**：`//` 开头的来源头注释（抓帧日期 + 形态 + 关键字段 + 抓法）；
- **其后**：单行真 Go 帧 JSON 原文（`{v,action,data,broadcast,seq,...}`）。

测试侧 `fixture_body()` 剥掉 `//` 行后拼回 JSON 喂解析器。
（注：标准 JSON 不支持注释，故用「头注释 + JSON 体」两段式；`all_fixtures_are_valid_json_after_stripping_header`
测试守护剥注释后必为合法 JSON。）

## 当前在册 fixture

| 文件 | action | 形态 / 关键字段 | 回放断言 |
|---|---|---|---|
| `post.json` | `post` | 新消息广播，event_seq=40114 / temporaryId / id / readBits="100" | 落 message 行 4 列 + cursor 推进 + emit |
| `increment_channel.json` | `increment_channel` | 冷启动增量，id=…fusnfa / lastEventSeq=151 / needSync=true | 注册 + cursor 种子 + emit |
| `increment_channel_end.json` | `increment_channel_end` | data={} → global end | 对增量群发 /channel/sync/notify |
| `status_change_ignored.json` | `status_change` | 运维帧 status=online | FrameClass::Ignored（零 effect/零 panic） |
| `connect_init.jsonl`（既有） | `hello` + 运维帧 | 握手 + status_change/multiple_channels_viewed | 见 `real_go_frames_test.rs` |
| `sync_no_change.json` ⚠️占位 | HTTP `no_change` | `/channel/sync/notify` reply，kind=no_change / nextSeq=maxSeq | `sync_http_fixtures_test.rs`：parse→NoChange |
| `sync_events.json` ⚠️占位 | HTTP `events` | reply，kind=events / eventSeq 单调 / nextSeq=末条 seq | parse→Events{next_seq,!needs_continuation} |
| `sync_too_long.json` ⚠️占位 | HTTP `too_long` | reply，kind=too_long / resetTo=maxSeq | parse→TooLong{reset_to}（清内存不清表） |

> ⚠️占位 = PLACEHOLDER：非真 Go 抓帧，按 `parser::parse_sync_response` 文档 wire 形态手填，头注释带
> 「待真抓」+ 真抓法。`sync_http_fixtures_test.rs` 喂**真 parser** 锁 wire 形态——真帧抓到覆盖文件后
> 测试无需改动即继续守护。这三个走 **HTTP**（非 WS RAW），WS dump 抓不到，须先在 driver HTTP 层
> 补 dump（见「已知缺口」），抓真 body 后覆盖占位。

## 怎么抓新真帧 + 加 fixture（工具链的「链」）

### Step 1 — 用 `HELIX_DUMP_FRAMES=1` 抓帧

跑带真 Go 后端的 dev 循环，开启 RAW 帧 dump：

`HELIX_DUMP_FRAMES=1` 打开 WS 入站帧 dump（driver `network.rs`，`target="helix_ws_frames"`，
日志行 `RAW WS inbound len=... frame={...}`）。
> HTTP 响应体（`/channel/sync/notify` sync reply）目前**不在** dump 内——见下方「已知缺口」，
> 需先补 driver HTTP 层 dump 才能抓真 sync body。

```bash
HELIX_DUMP_FRAMES=1 <你的 dev/driver 启动命令> 2>&1 | tee /tmp/helix_capture.log
```

driver 侧会把每条入站 WS 帧打成 `RAW WS inbound len=... frame={...}` 日志行
（target=`helix_ws_frames`）。触发你想抓的业务动作（发消息 / 冷启动 / 切群 …），
对应帧就会落进 `/tmp/helix_capture.log`。

### Step 2 — 从 log 抠 frame JSON 原文（剥 ANSI 颜色）

日志行有 ANSI 颜色码包裹，抠 `frame=` 之后的 JSON：

```bash
python3 - <<'PY'
import re, json
lines = open('/tmp/helix_capture.log').read().splitlines()
def frame_of(line):
    line = re.sub(r'\x1b\[[0-9;]*m', '', line)   # 剥 ANSI
    i = line.find('frame=')
    return line[i+6:].rstrip() if i >= 0 else None

for l in lines:
    f = frame_of(l)
    if not f:
        continue
    try:
        d = json.loads(f)
    except Exception:
        continue
    if d.get('action') == 'YOUR_ACTION':          # 改成你要抓的 action
        print(json.dumps(d, separators=(',', ':')))  # compact 单行
        break
PY
```

### Step 3 — 写 fixture 文件（头注释 + JSON 体）

新建 `tests/fixtures/real_go_frames/<name>.json`：

```text
// 来源: <log 路径> 真 Go 现网抓帧 (<日期>, <形态一句话>)
// 形态: action=="<action>"，<关键字段=值 列举>
// 抓法: HELIX_DUMP_FRAMES=1 跑 dev 循环，grep "RAW WS inbound ... frame=" 抠 frame JSON 原文
{"v":1,"action":"<action>","data":{...},"broadcast":{...},"seq":N}
```

### Step 4 — 加回放断言到 `golden_replay_test.rs`

1. `include_str!("fixtures/real_go_frames/<name>.json")` 注册常量；
2. 加进 `all_fixtures_are_valid_json_after_stripping_header` 的列表；
3. 写一个 `#[test]`：`h.step(inbound(<NAME_JSON>), now)` → 断言 Effect/落库/cursor/emit。
   - 确定性：channel 用真帧 `channelId`（`ChannelId::from_str(...)`）；
     需立即 apply 的 post 帧把 cursor 预种到 `event_seq-1`（`h.module.register_channel`）。

### Step 5 — 跑测试

```bash
cargo test -p helix-im --test golden_replay_test
```

## 已知缺口（待后续补真帧）

- **sync `no_change` / `too_long` / `events` HTTP 响应体**：这些走 **HTTP**（`/channel/sync/notify`
  的 reply body），不在 WS RAW 帧日志里。✅ P0 已落三个 ⚠️占位 fixture（`sync_no_change/sync_events/
  sync_too_long.json`）+ `sync_http_fixtures_test.rs` 喂**真 parser** 锁 wire 形态（parse→NoChange/
  Events/TooLong）。**剩余两步（P1+ 接力）**：① 在 driver HTTP 层（`network.rs` 的 `request()` 读到
  body 后）加 `HELIX_DUMP_FRAMES` 旁路 dump（对齐 WS dump，注意 `structure-gate.sh` 对 network.rs
  的 463 行 baseline——dump 落地需同步该 backlog 拆分或基线调整）；② 抓真 body 覆盖占位文件 + 删
  占位头注释 + 升级到走 `handle_sync_reply` 的落库/cursor/emit 集成断言（模板
  `c2_phantom_readbits_test.rs` / `c3_sync_mutation_emits_test.rs`）。
- **`post` echo 帧**（本端 send 回流，temporary_id 命中在途 PendingSend）：当前 `post.json`
  的 temporaryId 是本端注入但测试未注册 PendingSend；后续可抓「先 send 后 echo」的真帧对
  对账链路（reconcile + CancelTimer）补断言。
