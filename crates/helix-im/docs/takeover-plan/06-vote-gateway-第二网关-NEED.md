# NEED — vote/score 走「另一个网关 base」，现有 outbound 单 base 机制无法表达

> 状态：BLOCKED（不强行实现）。本族 5 命令（readVote/createVote/deleteVote/readAverageScore/deleteAverageScore）
> 与既有 88+ outbound 命令的**唯一结构差异 = HTTP 网关 base 不同**。现有 outbound 机制硬编码单 base，
> 表达不了第二 base → 需 core/driver 加一个「gateway base 维度」后才能正确落地。先记需求，不假装。

---

## 一句话

vote/score 在现网走**默认网关** `this.api.post$`（base = `env.restHost`，**无** `/api/cses` 前缀），
endpoint `/vote/*` `/average/*`；而 helix 既有 outbound 全部走 **IM 网关** `imHttp`
（base = `api_base_url` = `http://host:8065/api/cses`）。两者是**不同 base host**，现有机制只有单 base。

---

## 真源证据（cses-client，行为真源 §4.5）

`src/pages/message-v3/service/message.service.ts`：

| 注入器 | base 来源 | 用途 | 例 |
|---|---|---|---|
| `imHttp = inject(IM_HTTP)` | IM 域 host（含 `/api/cses`，`http-client-config.ts` 注释「imHttp 自动省略 /api/cses 前缀」）| 既有 88 命令 | `imHttp.post$('/posts/revoke', …)` |
| `api = inject(AppHttpService)` | **主业务域** `env.restHost`（**无** `/api/cses`，`app-http.service.ts:227 resolveUrl` 用 `cfg.host ?? env.restHost`）| **vote/score + search** | `api.post$('/vote/readVote', …)` |

- `docs/case/11-投票评分.md:119`：`全部走 http (不走 imHttp) 路径前缀: /vote/* 或 /average/*` —— 决定性。
- `IM_HTTP` 与 `APP_HTTP` 是「一个类两个实例」（`http-client-config.ts:9`），**base host 不同**。

5 命令 endpoint + body（真源同文件 951-1043 行，param type 34-100 行）：

| 命令名(建议) | endpoint（default 网关，不含 /api/cses）| body（camelCase 原样透传）|
|---|---|---|
| readVote | `vote/readVote` | `{ id }` |
| createVote | `vote/createVote` | `{ fromUserId, fromUserName, title, content, votes, isReal, finishTime, options[], orgIds[], joinCount, source? }` |
| deleteVote | `vote/deleteVote` | `{ id }` |
| readAverageScore | `average/read` | `{ id }` |
| deleteAverageScore | `average/delete` | `{ id }` |

readVote / readAverageScore = 读族（`is_read=true`）；create/delete = 写族（默认 `is_read=false`）。

---

## 为什么现有机制表达不了（机制判定）

落点逐层：

1. **`OutboundCommand::build`**（`registry.rs:34`）只返回 `(&'static str endpoint, Value body)`——**无 base 维度**。
2. **`http_post`**（`registry.rs:166-188`）固定 `url: format!("{api_base_url}/{path}")`——`api_base_url` 是
   `handle_outbound` 唯一入参，由 `module.rs:1063` 传 `self.config.api_base_url`（= IM 网关含 `/api/cses`）。
   vote 走它 = `http://host:8065/api/cses/vote/readVote` ❌（多了 `/api/cses`）。
3. **`Effect::Http.req.url`**（`effect.rs:282`）是已拼好的完整字符串——core 此处已定 base，无「切 base」开关。
4. **driver `resolve_http_url(base, req_url)`**（`network.rs:438`）：**若 `req_url` 已是 absolute URL 则原样返回**，
   否则 `base.join(req_url)`。driver 用单一 `config.api_base_url`（`HostNetworkConfig` 只一个 base 字段）。
5. **config**：`ImConfig`（`module.rs:24`）与 `HostNetworkConfig`（`network.rs:32`）都**只有一个 `api_base_url`**，
   没有 default-gateway base。

结论：**无歧义判定——现有单 base outbound 机制无法表达 vote/score 的第二 base，需扩维。** 三条候选解，
各有取舍，需用户/驱动层定夺（涉及 core Effect 或 config 形状，超 outbound 单文件白名单范围）：

### 方案 A（推荐）— config 加 `default_api_base_url` + 命令标 gateway 意图

- `ImConfig` / `HostNetworkConfig` 加第二字段 `default_api_base_url`（host 注入 = `env.restHost`，无 `/api/cses`）。
- `OutboundCommand` 加 `fn gateway(&self) -> Gateway { Gateway::Im }`（enum `Im | Default`，默认 Im 零 ripple）；
  vote/score override `Gateway::Default`。
- `handle_outbound` 接两个 base，`http_post` 按 `gateway()` 选 base 拼 url。
- driver `HostNetworkConfig` 加 `default_api_base_url` + `SharedHttpClient` 不变（url 已在 im 拼好）。
- 优点：core 不硬编码 host（HX-C001 守住，base 仍由 host 注入，确定性）；与既有 `AuthKind` 意图维度同构
  （命令标「意图」、driver 给真值），最小惊讶；88 既有命令零改动。
- 影响面：`effect.rs` 不变；`ImConfig`/`HostNetworkConfig` 各 +1 字段；`registry.rs` http_post 选 base；
  host-cli + Tauri 接缝注入第二 base（= 把现有 base 去掉 `/api/cses` 尾段）。

### 方案 B — 命令直接产 absolute URL（利用 driver 现有 absolute 短路）

- vote/score 的 `build`/`path_override` 产完整 `http://…/vote/readVote`，借 `resolve_http_url` absolute 短路。
- ❌ 否决：core 里出现具体 host 字符串 = **破 HX-C001（core 零平台/零 host 硬编码）+ 破确定性**
  （host 不能由 core 拼；登录态/环境不同 host 不同）。不可取。

### 方案 C — core 里把 `api_base_url` strip 掉 `/api/cses` 尾段

- `http_post` 对 default-gateway 命令做 `api_base_url.strip_suffix("/api/cses")`。
- ❌ 否决：core 耦合「IM base 必以 /api/cses 结尾」这一 path 约定（脆，host 改 base 形状即碎）；
  且把「网关拓扑」知识塞进 core 状态机（应在 host/config）。违 §依赖倒置 + 边界零信任。

---

## 当前处置

- **不实现 vote_score.rs**（强行用方案 B/C 会破 HX-C001/确定性，留技术债更糟）。
- 待方案 A（或等价的 driver/config 扩维）拍板后，再落 `vote_score.rs`（届时 body/endpoint/读写族判定
  已全部就绪，见上表，纯补 `gateway()` override + 注册即可，预计 < 1 文件）。
- 本族 5 命令在 outbound 总数对账中**暂不计入**（注册表 count 不含）。

## 给主对话/驱动 worktree 的接力

实现方案 A 时改动清单（跨 im + core/driver + host 接缝，非 outbound 单文件白名单）：

1. `helix-core/src/effect.rs`：**无需改**（url 仍在 im 拼好）。
2. `helix-im/src/module.rs` `ImConfig` + `helix-driver-host/src/network.rs` `HostNetworkConfig`：各加
   `default_api_base_url: String`。
3. `helix-im/src/outbound/registry.rs`：`OutboundCommand` 加 `gateway()`；`handle_outbound` 增第二 base 入参；
   `http_post` 按 gateway 选 base。
4. `helix-im/src/outbound/vote_score.rs`（新建）：5 命令，各 `gateway()→Default`，read 两个 `is_read()→true`。
5. host-cli / Tauri 接缝：注入 `default_api_base_url`（= 去掉 `/api/cses` 的同 host）。
6. 对账：outbound count 期望值 +5。

> 完成后在本 worktree 写 `RESOLVED_vote_gateway.md` 告知可切真实现。
