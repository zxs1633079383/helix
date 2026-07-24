# Rich/File 消息上传发送设计

日期：2026-07-24
状态：Rust/Java/Go 本地实现及 host/HTTP/FFI 合同验收已完成；产品前端接线、64 MiB v1
边界确认、Java 发布部署与真实环境全链路仍需验收

## 已拍板拓扑

前端分别提供两个互不推导的地址：

- `api_base_url`：Go IM 地址，只承接 `/api/cses/posts/create`、同步与消息接口。
- `default_api_base_url`：Java `env.restHost`，媒体 prepare/verify/稳定读取都走它。

媒体发送链固定为：

```text
UI 选文件
  → Helix PersistAtomic(message + pending_media prepare journal)
  → Rust → Java POST /oss/getPresignedUrl
  → Rust host → OSS PUT
  → Rust → Java POST /oss/completeMediaUpload
  → Rust → Go POST /api/cses/posts/create
```

Rust 不从 Go 地址裁剪或猜出 Java 地址。Web 配置显式提供两个 base；FFI 使用
`helix_create_with_endpoints(go_api_base, java_api_base, ws_url, ...)`。旧 FFI 网络构造入口
只保留 ABI 兼容，不具备 Java 地址，媒体发送必须 fail-closed。

## 本轮平台验收范围

- 本轮真实媒体上传只验收 native/FFI：二者复用 shared host 文件上传 driver，可读取宿主
  `localPath` 并执行 OSS PUT。
- Web 的 `apiBaseUrl/defaultApiBaseUrl` 双地址配置槽已经存在，浏览器 bootstrap 必须分别
  注入 Go 与 Java 地址；这只证明控制面配置边界，不代表浏览器文件上传已经实现。
- `helix-driver-web` 当前对 `Effect::UploadFile` 明确返回 unsupported，并且 runtime issue
  不记录本地路径。Web rich/file 上传不属于本轮可用性声明；后续需先增加浏览器
  `File/Blob` 输入、上传 port 与 CORS 验收，才能纳入真实媒体 E2E。

## 输入与服务端所有权

UI 只提交本地文件事实：

```json
{
  "mediaInput": {
    "localPath": "/tmp/report.pdf",
    "fileName": "report.pdf",
    "contentType": "application/pdf",
    "size": 8388608,
    "sha256": "<64位小写hex>"
  }
}
```

UI 不得提交 `uploadToken/fileId/objectKey/url/stablePath/headers/state`。Helix 用业务消息
`temporaryId + target(file|rich:index)` 生成稳定 `clientUploadId`；Java 结合登录 `userId`
和服务端密钥派生不可预测、owner-bound 的 `fileId` 与 `objectKey`，客户端无权选择
bucket、key 或 TTL。

`mediaInput` 只用于建模内部上传任务，进入乐观消息前整体移除；兼容输入中的
`localUrl/local_url/localPath/local_path` 也必须从 `props.file/props.files[]` 及其
`upload` 子对象剥离。内部 `pending_media.local_path` 可为崩溃恢复保存宿主路径，但它不得
进入 message props、Go `posts/create`、DomainEvent、trace 或 Web runtime issue。

## Java prepare 合同

```http
POST {javaBase}/oss/getPresignedUrl
X-Auth-Kind: session
Content-Type: application/json
```

```json
{
  "mediaUpload": true,
  "clientUploadId": "helix_tmp_x:file",
  "fileName": "report.pdf",
  "contentType": "application/pdf",
  "size": 8388608,
  "sha256": "<64位小写hex>"
}
```

Java raw response：

```json
{
  "isSuccess": true,
  "method": "PUT",
  "uploadToken": "<绑定用户、clientUploadId、对象身份与完整性声明的 HMAC 票据>",
  "fileId": "mf_<23位小写base36>",
  "objectKey": "media/mf_<23位小写base36>",
  "url": "<10分钟 PUT 预签名地址>",
  "stablePath": "/oss/media/object/mf_<23位小写base36>",
  "expiresAt": 0,
  "headers": {
    "Content-Type": "application/pdf",
    "x-oss-forbid-overwrite": "true"
  }
}
```

Rust 只接受 `method=PUT`、`objectKey=media/{fileId}`、
`stablePath=/oss/media/object/{fileId}`、非空有界 `uploadToken` 和两条精确必需 header。
预签名 PUT URL 与 headers 只存在于进程内，禁止写 SQLite、事件、trace 或最终消息 props。
`uploadToken` 只进入内部 `pending_media`，默认日志必须脱敏，公开消息 props 不得携带。

## PUT 与 verify 合同

shared host driver 流式读取 `localPath` 并对 prepare URL 执行 PUT。成功后：

```http
POST {javaBase}/oss/completeMediaUpload
X-Auth-Kind: session
Content-Type: application/json
```

```json
{
  "mediaUpload": true,
  "clientUploadId": "helix_tmp_x:file",
  "uploadToken": "<prepare 返回的 HMAC 票据>",
  "fileId": "mf_x",
  "objectKey": "media/mf_x",
  "contentType": "application/pdf",
  "size": 8388608,
  "sha256": "<64位小写hex>"
}
```

Java 重新计算 owner-bound 身份并常量时间校验 `uploadToken`，使 verify 的 size/type/SHA
不能脱离 prepare 声明被篡改；随后从 OSS 读取真实 metadata/对象字节核对 size、
Content-Type 与 SHA-256。全部通过后 Java 才写服务端 `media-completed/{fileId}` 标记。
Rust 只接受 `isSuccess=true,state=completed` 且所有字段与 prepare、本地 journal 完全一致；
否则 fail-closed，禁止调用 Go `posts/create`。

最终消息只保存稳定地址：

```text
{javaBase}/oss/media/object/{fileId}
```

该地址只有在 completed 标记存在时才由 Java 返回 302 到 5 分钟 GET 预签名 URL；未 verify
或校验失败对象不可读。Java 仅允许安全栅格图片 inline，其余类型强制 attachment 并覆盖为
`application/octet-stream`。消息、SQLite 与 Go 数据库永不保存短时 GET/PUT 签名。

## 并发、进度与大文件

- rich 最多九张图。九个 prepare Effect 在同一批次吐给 host，由 I/O 层并发执行，不逐张
  等待上一张完成。
- 图片不发逐图百分比；整条 rich 消息在全部图片 verify 完成前保持 Sending/转圈。
- FILE 保留 5% 粒度进度；进度由 driver 已上传字节产生，经 Helix 投影回前端。
- PUT 已完成但某个较早的进度 Persist 尚未回执时，终态推进必须等待该回执；否则不同
  storage worker 的乱序完成可能让旧百分比覆盖 100%。进度写失败不阻断最终 verify，
  最终完成持久化仍以 100% 收口。
- 当前 Helix v1 媒体上限 64 MiB，使用流式单 PUT，内存不会按文件大小整体加载。
- 现网客户端的 65 MiB+ multipart/checkpoint 能力不在本轮等价范围；若产品验收要求大文件，
  必须在 driver port 增加 multipart/checkpoint 合同后再切换，不能只放大单 PUT 上限。

## 重试与持久化

- Prepare 失败：使用同一 `clientUploadId` 重试 prepare。
- PUT 失败或回包不确定：回到 Prepare。Java 若发现同 owner/clientUploadId 下存在无 completed
  标记的 pending 对象，先删除该未可信对象，再签发新的 forbid-overwrite URL；旧签名永不复用。
- 已有 completed 标记的对象不可再次 Prepare 覆盖，稳定消息引用保持不可变。
- Verify 失败：持久化 fileId/objectKey/稳定 URL，只重复 verify，不重复 PUT。
- `pending_media` 可以持久化稳定身份、HMAC `uploadToken` 和阶段，但 `upload_url` 必为空、
  `headers_json` 必为空数组；token 必须在 storage trace 与 HTTP debug 中脱敏。
- 媒体消息首次落库必须用一笔 `PersistAtomic` 同时提交 message 与 prepare journal。
  若该事务失败，失败态补偿也必须原子提交 `unsend` message 与原始 prepare journal；
  进程在补偿后重启，retry 只能恢复 journal 并重新请求 Java prepare，禁止把未校验 props
  直接发给 Go。
- 所有附件 verify 后才恰好一次调用 Go `posts/create`；重试不生成新的业务消息 ID。

## 验收底线

- Go 与 Java 使用不同 origin 时，prepare/verify 命中 Java，`posts/create` 命中 Go。
- 九图 prepare 同批产生，图片无进度事件；FILE 仍有 5% 进度与 100% 终态。
- 任一 uploadToken/verify 字段缺失、篡改或冲突都不得发送消息。
- 未完成对象的稳定 GET 必须失败；完成标记存在后才允许 302，且主动内容不得 inline。
- 重启 Complete 阶段不重复 PUT，且任何持久化/日志中没有签名查询串。
- 真实 OSS 验收必须覆盖 rich 与 FILE：PUT、verify、稳定 URL 302、最终 GET 的长度、
  Content-Type、SHA-256，以及 Go 保存/WS 回声中的稳定 props。
