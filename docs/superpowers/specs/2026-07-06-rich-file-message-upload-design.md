# Rich/File 消息上传发送设计

日期：2026-07-24
状态：Rust/Java/Go 本地实现及 host/HTTP/FFI 合同验收已完成；产品前端接线、64 MiB v1
边界确认、Java 发布部署与真实环境全链路仍需验收

## 已拍板拓扑

前端分别提供两个互不推导的地址：

- `api_base_url`：Go IM 地址，只承接 `/api/cses/posts/create`、同步与消息接口。
- `default_api_base_url`：Java `env.restHost`，只用于媒体 PUT 票据签发。

媒体发送链固定为：

```text
UI 选文件
  → Helix PersistAtomic(message + pending_media prepare journal)
  → Rust → Java POST /oss/getPresignedUrl
  → Rust host → OSS PUT
  → Helix PersistAtomic(bucket + uri + delete pending_media)
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

UI 不得提交 `bucket/fileId/objectKey/uri/url/headers/state`。Helix 用业务消息
`temporaryId + target(file|rich:index)` 生成稳定 `clientUploadId`，并根据目标选择
`attachment/picture` bucket；Java 返回该 bucket 下的 `fileId/objectKey/uri` 与短时 PUT
票据，客户端无权选择 key 或 TTL。

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
  "bucket": "attachment",
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
  "fileId": "mf_<23位小写base36>",
  "bucket": "attachment",
  "objectKey": "media/mf_<23位小写base36>",
  "uri": "media/mf_<23位小写base36>",
  "url": "<10分钟 PUT 预签名地址>",
  "headers": {
    "Content-Type": "application/pdf",
    "x-oss-forbid-overwrite": "true"
  }
}
```

Rust 只接受 `method=PUT`、目标匹配的 bucket、`objectKey == uri == media/{fileId}` 和两条
精确必需 header。预签名 PUT URL 与 headers 只存在于进程内，禁止写 SQLite、事件、trace
或最终消息 props。

## PUT 与本地 Complete 合同

shared host driver 流式读取 `localPath` 并对 prepare URL 执行 PUT。OSS 返回 2xx 后，Helix
不再调用 Java complete/verify；它先将 `bucket + uri`、fileId、文件名、类型、大小与原始
SHA 写入 message props，同时删除对应 `pending_media` journal。该变化必须使用同一笔
`PersistAtomic`，只有 PersistOk 后才可推进下一附件或最终 Go `posts/create`。

最终消息引用形态：

```json
{
  "bucket": "attachment",
  "id": "mf_x",
  "uri": "media/mf_x",
  "name": "report.pdf",
  "contentType": "application/pdf",
  "size": 8388608,
  "sha256": "<64位小写hex>",
  "upload": {"status": "completed"}
}
```

消息、SQLite 与 Go 数据库永不保存短时 PUT URL、签名 headers 或本地路径。

## 并发、进度与大文件

- rich 最多九张图。九个 prepare Effect 在同一批次吐给 host，由 I/O 层并发执行，不逐张
  等待上一张完成。
- 图片不发逐图百分比；整条 rich 消息在全部图片 Local Complete 前保持 Sending/转圈。
- FILE 保留 5% 粒度进度；进度由 driver 已上传字节产生，经 Helix 投影回前端。
- PUT 已完成但某个较早的进度 Persist 尚未回执时，终态推进必须等待该回执；否则不同
  storage worker 的乱序完成可能让旧百分比覆盖 100%。进度写失败不阻断最终 Local Complete，
  最终完成持久化仍以 100% 收口。
- 当前 Helix v1 媒体上限 64 MiB，使用流式单 PUT，内存不会按文件大小整体加载。
- 现网客户端的 65 MiB+ multipart/checkpoint 能力不在本轮等价范围；若产品验收要求大文件，
  必须在 driver port 增加 multipart/checkpoint 合同后再切换，不能只放大单 PUT 上限。

## 重试与持久化

- Prepare 失败：使用同一 `clientUploadId` 重试 prepare。
- PUT 失败或回包不确定：回到 Prepare，旧签名永不复用。
- PUT 2xx 后：持久化 Complete 阶段；Local Complete 失败或重启只重试本地原子提交，不重复 PUT。
- `pending_media` 可以持久化稳定 fileId/objectKey/bucket/uri 和阶段，但 `upload_url` 必为空、
  `headers_json` 必为空数组。
- 媒体消息首次落库必须用一笔 `PersistAtomic` 同时提交 message 与 prepare journal。
  若该事务失败，失败态补偿也必须原子提交 `unsend` message 与原始 prepare journal；
  进程在补偿后重启，retry 只能恢复 journal 并重新请求 Java prepare，禁止绕过状态机把 props
  直接发给 Go。
- 所有附件 Local Complete 的 PersistOk 到达后才恰好一次调用 Go `posts/create`；重试不生成新的业务消息 ID。

## 验收底线

- Go 与 Java 使用不同 origin 时，只有 prepare 命中 Java，`posts/create` 命中 Go。
- 九图 prepare 同批产生，图片无进度事件；FILE 仍有 5% 进度与 100% 终态。
- 任一 bucket/fileId/objectKey/uri/method/required header 缺失、篡改或冲突都不得开始 PUT。
- 重启 Complete 阶段不重复 PUT，且任何持久化/日志中没有签名查询串。
- 真实 OSS 验收必须覆盖 rich 与 FILE：PUT 2xx、本地原子 Complete、Go 保存/WS 回声中的
  `bucket + uri` 稳定 props，以及最终 Frame typed media items。
