# Web W0 浏览器可达性与鉴权验证执行方案

> 状态：ready-for-execution（2026-07-13；开发分支 `codex/web-w0-development`）
>
> 上位决策：[`../harness/decisions.md`](../harness/decisions.md) ADR-014
>
> 进度入口：[`../ROADMAP.md`](../ROADMAP.md) M4/W0
>
> 本文是 W0 期间唯一执行入口。W0 结束后，最终路线写入新的 ADR，ROADMAP 回写结果；本文只保留仍有复用价值的浏览器验证契约，删除一次性过程记录。

## 0. 结论先行

W0 不开发 `helix-driver-web`，也不实现 Web 持久化。它只回答一个会决定后续路线的问题：

> 真实浏览器能否在当前部署与鉴权约束下，完成受保护 HTTP 请求、建立 WebSocket、收到 `hello`，并在不泄露长期凭据的前提下稳定重连？

只有拿到真实浏览器证据后，才能在以下路线中拍板：

1. 浏览器直连 Go，使用浏览器可自然携带的 Cookie/session；
2. 浏览器直连 Go，先换取短期一次性 WS ticket；
3. 同源 BFF/WS gateway 代浏览器注入现有握手 header；
4. 连接建立后发送认证帧（需要服务端明确支持）；
5. 受约束地使用 `Sec-WebSocket-Protocol`（仅在协议与日志安全验证通过后考虑）。

W0 **不替用户选择路线**。任何需要修改 Go、网关、部署或登录协议的动作，都必须先提交事实证据、选项、风险和推荐理由，等待用户拍板。

## 1. 已核实的代码基线

### 1.1 可直接复用的核心契约

代码知识图已核实：

- `helix-core::ports::Transport` 已提供 `connect(&mut self)`、`send/recv/close(&self)`；
- `Storage` 已提供 batch upsert/update/delete、monotonic upsert、get、scan 等完整行为面；
- `HttpRequester` 只要求 `request(HttpRequest) -> HttpResponse`；
- `EventSink::emit` 是同步、非阻塞入队契约；
- `helix-im` 已消费 `Tick::Connected/Inbound/Disconnected`，并维护 hello、sequence/cursor、echo 与补偿同步。

因此 W1/W2 的目标是为浏览器兑现现有 ports，而不是改 core 或复制 IM 状态机。W0 不应提出新的 core port。

### 1.2 现有 host 网络实现不能直接搬到浏览器

`SharedWsClient::connect` 当前通过 `tokio-tungstenite` 构造 upgrade request，并把 `HostHeaderRegistry` 中的 header 全部注入握手；这适用于 native/FFI，但浏览器 `WebSocket` 构造面不提供等价的任意握手 header 能力。

当前 session auth 事实是：

- HTTP cross-cutting 层把 session intent 转成实际 `cookieId` header；
- native WS 同样依赖握手 header registry；
- `deviceId/companyId/appType` 等身份材料也可能参与现网校验；
- HTTP `fetch` 可以尝试自定义 header，但跨域时会触发 CORS preflight；
- 浏览器 WS 能否携带有效身份，取决于同源 Cookie、ticket、subprotocol、认证帧或 BFF，而不是 Rust 是否能创建 `web_sys::WebSocket`。

### 1.3 历史 wasm crate 只作反例与片段参考

`crates/helix-driver-wasm` 当前事实：

- `WasmTransport` 只有 URL，WebSocket 字段和收帧 channel 仍是 TODO；
- `WasmStorage` 没有真实数据库，且仍围绕历史 OPFS/SQLite 设想；
- engine loop 只有 `spawn_local` 空壳；
- `WasmHttp`、`WasmEventSink` 未兑现；
- 仅 `JsClock` 有可用实现片段。

W0 禁止在该 crate 上补 TODO。W1 会新建正式 `helix-driver-web`，再逐片段审计是否迁移。

## 2. W0 范围

### 2.1 必须完成

- 在真实浏览器记录 HTTP same-origin 与 cross-origin 的预检和实际响应；
- 建立浏览器 WebSocket，记录 `open/message/error/close`；
- 判断是否收到真实 Go `hello`，而非只证明 TCP/upgrade 成功；
- 验证刷新、离线/在线切换、服务端主动断开后的行为；
- 产出不含秘密值的结构化证据；
- 给出鉴权路线对比和推荐，但在用户拍板前不改服务端或部署。

### 2.2 明确不做

- 不新建 `helix-driver-web`；
- 不编译 wasm target，不激活 HX-C003；
- 不实现 IndexedDB、OPFS、SQLite wasm 或 cursor 恢复；
- 不修改 `helix-core` / `helix-im`；
- 不修改 Go/BFF/网关；
- 不把 token、Cookie、`cookieId`、完整 WS URL query 或请求 header 写入证据文件；
- 不用“WebSocket open 成功”代替“收到 hello 且身份正确”。

## 3. 前置输入

### G0：真实环境输入

执行者必须拿到以下运行时输入，但不得提交到仓库：

- Web 页面 origin；
- Go API base URL；
- Go WS URL；
- 一个可登录的测试账号；
- 现网登录流程产生的浏览器 session/Cookie，或临时测试凭据；
- `deviceId/companyId/appType` 的真实来源说明；
- 浏览器矩阵：至少 Chrome 当前版本，正式验收再补 Safari/Firefox。

缺任一项时可以完成 probe 工具，但不能宣称 W0 通过。

## 4. 预期新增的最小探针

W0 实现阶段只新增一个无框架、无依赖的浏览器探针：

```text
tools/web-w0-probe/
├── index.html                 # 输入非秘密 URL/模式，展示结构化结果
├── probe.mjs                  # fetch + WebSocket + 生命周期编排
├── probe-model.mjs            # 纯函数：结果分类、脱敏、验收判定
├── probe-model.test.mjs       # node:test，不启动浏览器
└── README.md                  # 启动、登录前置、人工步骤、禁提交秘密
```

不新增 npm 依赖和 `package.json`。本地启动命令固定为：

```bash
python3 -m http.server 4173 --directory tools/web-w0-probe
```

纯模型测试：

```bash
node --test tools/web-w0-probe/probe-model.test.mjs
```

浏览器自动化只负责打开页面、点击 probe 和保存脱敏结果；登录态建立若依赖真实 SSO，允许人工完成，不能把账号密码写入脚本。

## 5. 结构化证据合同

探针只能导出以下结构，所有秘密值必须在内存中使用、导出前删除：

```json
{
  "schema": "helix.web-w0-probe.v1",
  "caseId": "cross-origin-existing-session",
  "browser": {
    "name": "Chrome",
    "version": "major.minor",
    "platform": "macOS"
  },
  "pageOrigin": "http://127.0.0.1:4173",
  "apiOrigin": "https://example.invalid",
  "wsOrigin": "wss://example.invalid",
  "http": {
    "preflightObserved": true,
    "status": 200,
    "corsClass": "allowed-with-credentials",
    "bodyClass": "authenticated-success",
    "elapsedMs": 0
  },
  "websocket": {
    "opened": true,
    "helloReceived": true,
    "firstAction": "hello",
    "closeCode": 1000,
    "closeReasonClass": "normal",
    "elapsedMs": 0
  },
  "lifecycle": {
    "reload": "reconnected",
    "offlineOnline": "reconnected",
    "backgroundForeground": "observed"
  },
  "secretsRedacted": true
}
```

禁止字段：

- `headers` 原文；
- `cookie` / `cookieId` / token 值；
- 完整响应 body；
- 携带 query ticket 的完整 URL；
- 用户名、密码、真实消息内容；
- WS 原始帧全文。

允许保存：状态码、CORS 分类、WS action 名、close code、耗时、是否收到 hello，以及经过白名单处理的错误类别。

## 6. 执行任务

### Task W0.0：开发基线确认

文件：`SESSION.md`。

步骤：

1. 确认当前分支为 `codex/web-w0-development`；
2. 区分并保护其他会话未提交修改；
3. 用知识图重新读取 `Transport`、`HttpRequester`、`Storage`、`EventSink`；
4. 记录 W0 实现 EXPECT，预测只新增 probe 文件，不碰 Rust；
5. `git diff --check`。

提交边界：治理 review 独立提交，不与 probe 混合。

### Task W0.1：先写 probe 纯模型测试

文件：

- 新增 `tools/web-w0-probe/probe-model.mjs`；
- 新增 `tools/web-w0-probe/probe-model.test.mjs`。

测试必须覆盖：

- HTTP 2xx + hello → `pass`；
- WS open 但无 hello → `fail-auth-or-protocol`；
- preflight 被拒 → `fail-cors`；
- close 1006 → `fail-abnormal-close`；
- 401/403 → `fail-auth`；
- 导出对象出现敏感字段名或疑似 token → 测试失败；
- 同一 case 缺少 page/api/ws origin → 测试失败。

验收：`node --test ...` 全绿，并人工破坏一条分类断言证明测试可证伪。

提交边界：只含纯模型与测试。

### Task W0.2：实现 HTTP 浏览器探针

文件：

- 新增 `tools/web-w0-probe/index.html`；
- 新增 `tools/web-w0-probe/probe.mjs`；
- 新增 `tools/web-w0-probe/README.md`。

默认只读探针请求：

```text
POST <apiBase>/notification/loadTarget
Content-Type: application/json
body: {"page":0,"size":10}
```

理由：这是现有 `im_load_target_notifications` 已覆盖的 session-auth 读取路径，不会创建/编辑消息。若目标环境未开放该接口，必须从当前 Strict Matrix 选择另一个只读 session endpoint，并把替换理由写入结果；禁止临时改用发消息、撤回、成员变更等写接口。

身份值只允许放在页面内存中的 password 类型输入控件，禁止写入 URL、localStorage、sessionStorage、IndexedDB 或默认配置。H1 same-origin case 只有在环境已经能把 probe 静态页挂到 API 同源入口时才执行；若不存在该入口，标记 `not-available`，W0 不为凑 case 临时实现 BFF。

至少跑四个 case：

| case | 页面与 API | credentials | 自定义身份 header | 目的 |
|---|---|---|---|---|
| H1 | same-origin | include | 无 | 判断同源 Cookie/session 是否足够 |
| H2 | cross-origin | omit | 无 | 记录匿名基线 |
| H3 | cross-origin | include | 无 | 验证 credentialed CORS |
| H4 | cross-origin | include | 当前真实必需 header | 验证 preflight 是否允许现网 header 集 |

必须记录：OPTIONS/实际请求是否发出、状态码、`Access-Control-Allow-Origin` 分类、credentials 分类、错误类别。不得记录 header 值。

### Task W0.3：实现 WebSocket 浏览器探针

使用浏览器原生 `WebSocket`，不引入 Rust/WASM。记录：

- 构造是否同步失败；
- `open` 是否触发；
- 第一条业务 action；
- 是否在超时内收到 `hello`；
- `error/close` 顺序、close code、脱敏 reason；
- reload、offline→online、background→foreground 后的行为。

至少跑三类连接：

| case | 方式 | 当前是否需服务端改动 | 判定 |
|---|---|---:|---|
| W1 | 直接 WS URL + 浏览器已有 Cookie | 否 | 能收到 hello 才算成功 |
| W2 | same-origin BFF 路径（若环境已有） | 否/配置 | 由 BFF 代注入 header，收到 hello |
| W3 | 无有效身份匿名连接 | 否 | 应明确失败，用于对照 |

ticket、subprotocol、认证帧只列为候选，不在 W0.3 擅自实现。若当前环境不存在 BFF，则 W2 标 `not-available`，不能伪造 pass。

### Task W0.4：生成鉴权路线决策包并暂停

基于 H1-H4/W1-W3 的真实结果，形成一页决策表：

| 路线 | HTTP 变化 | WS 变化 | Go 变化 | 部署变化 | 安全风险 | 推荐条件 |
|---|---|---|---|---|---|---|
| 同源 Cookie/session | 可能仅 CORS/Domain | Cookie 握手 | 登录态适配 | 同域/反代 | SameSite/CSRF | 已有安全 Cookie 时优先 |
| 短期 WS ticket | 取票接口 | query 或一次性协议 | 新增取票/消费 | 较小 | URL 日志与重放 | 无法同域时候选 |
| BFF/WS gateway | 浏览器同源 | gateway 注入 header | 可零改或小改 | 新增网关职责 | 代理信任边界 | 保留现网协议时推荐 |
| 连接后认证帧 | 可不变 | 新认证 action | WS 状态机改动 | 较小 | 未认证窗口 | Go 明确支持时考虑 |
| Subprotocol | 可不变 | 子协议传递 | 握手解析改动 | 较小 | 日志/兼容性 | 仅受控短期材料 |

输出必须包含：

- 事实证据；
- 至少两个可行选项；
- 推荐方案与反对方案；
- 服务端/部署影响；
- 回滚方式；
- 未决问题。

然后**停止**，向用户同步并等待拍板。禁止在同一提交里顺手修改 Go、BFF 或登录协议。

### Task W0.5：用户拍板后的决策落账

用户明确选择后：

1. 在 `docs/harness/decisions.md` 新增 ADR-015，冻结浏览器鉴权路线；
2. 更新 `docs/ROADMAP.md`：W0 标记完成或列出阻塞；
3. 若选项涉及跨仓修改，另写对应仓的执行方案与验证矩阵；
4. 若 W0 通过，再启动 W1 `helix-driver-web` 计划；
5. 若 W0 不通过，不进入 WASM/持久化实现。

## 7. W0 验收标准

以下条件全部满足才允许把 W0 标绿：

- [ ] probe 纯模型测试通过且有破坏即失败证据；
- [ ] 至少一个真实浏览器 HTTP case 得到 authenticated success；
- [ ] 至少一个真实浏览器 WS case 收到 Go `hello`；
- [ ] 匿名/失败对照 case 能区分 CORS、auth、protocol、network；
- [ ] refresh 和 offline→online 行为有实际记录；
- [ ] 导出证据不包含秘密值或真实业务内容；
- [ ] 用户已在候选路线中明确拍板；
- [ ] ADR-015 与 ROADMAP 已回写；
- [ ] 没有新建 `helix-driver-web`、没有实现 Storage、没有修改 core/im。

## 8. 提交序列

建议严格按以下提交拆分：

1. `test(web): 固化 W0 探针分类与脱敏合同`
2. `feat(web): 增加浏览器 HTTP 与 WebSocket 可达性探针`
3. `docs(web): 记录 W0 真实证据与鉴权候选`
4. 用户拍板后：`docs(web): 冻结浏览器鉴权路线`

任何 Go/BFF/部署修改必须在另一个仓库、另一个已确认计划中执行，不混进上述提交。

## 9. 失败与停止条件

命中任一项立即暂停并同步用户：

- HTTP 5xx、timeout、DNS/证书/代理或部署异常；
- 实际环境与文档中的 API/WS URL 不一致；
- 只有写接口可用于验证 session；
- 需要长期 token 放 query/localStorage；
- 需要修改 Go/BFF 才能继续；
- 真实 session 来源不明确；
- 证据无法在不保存秘密的情况下复现；
- 新发现的事实证明当前浏览器直连路线不可行，且继续需要跨仓或部署决策。

W0 的成功不是“写出一个 WebSocket demo”，而是把浏览器鉴权与连接可达性从猜测变成可复现、可拍板的证据。

## 10. 证据落点与文档熵管理

- 原始脱敏 JSON、截图和浏览器网络导出只放 `outputs/web-w0/`，保持未跟踪；
- 仓库内只在本文追加 H1-H4/W1-W3 的汇总矩阵，不提交 HAR、Cookie 或原始帧；
- 用户拍板后，最终路线进入 ADR-015，ROADMAP 回写 W0 结果；
- W0 完成时把本文收敛为“浏览器可达性验证契约”，删除一次性操作流水，不再新建 report/plan 目录。
