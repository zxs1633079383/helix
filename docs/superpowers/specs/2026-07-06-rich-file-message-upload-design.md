# Rich/File 消息上传发送设计

日期：2026-07-06
状态：设计稿，待实现计划

## 背景

当前 `im_send_message` 只覆盖文本发送主链路：

1. `helix-im` 组装 `posts/create` body。
2. 本地 `message` 表按 `temporary_id` 乐观落库，`send_status="sending"`。
3. driver 执行 `Effect::Http` 调用 `POST /posts/create`。
4. HTTP success 或 WS `action=="post"` echo 将 `send_status` 对账为 `"sent"`；失败或超时写 `"unsend"`。

新需求：

- 图片可以和文字组成同一条消息记录。
- 文件消息只能单独发送，不能和文字混合。
- 图片和文件都要先用前端传入的预签名信息上传到阿里云 OSS。
- 剪切板内容由前端或壳层先落成临时文件，再把本地路径传给 Rust。
- 前端调用 Rust 发消息时，会同时传 post 信息、文件基础信息和预签名上传信息。
- 本地库必须先有消息记录，再执行文件上传；上传成功后才调用服务端 `posts/create`。

## 非目标

- 不在 Helix 内生成预签名 URL。
- 不在 Helix 内保存阿里云 AK/SK。
- 不新增 `attachments` 作为 post 顶层字段。
- 不扩展现有 Post 发送状态枚举。
- 第一版不实现进行中上传取消。
- 第一版不设计分片上传、断点续传或秒传。

## 消息类型

沿用 `im_send_message`，通过 `type` 区分：

- `type="rich"`：文本加图片，或纯图片。
- `type="file"`：单文件消息。
- 现有文本消息仍可继续使用 `type="TEXT"` 或缺省类型。

`rich` 规则：

- `props.images` 可以有 1..N 张图片。
- `message` 可以为空，也可以有文本。
- `props.textPosition` 必须是 `"top"` 或 `"bottom"`。
- 不允许出现 `props.file`。
- 文本只能在全部图片上方或下方，不支持穿插在图片中间。

`file` 规则：

- 必须有 `props.file`。
- 不允许出现 `props.images`。
- `message` 必须为空；文件名、大小等展示信息放在 `props.file` 内。

## Props 契约

`props` 是开放 JSON。前端可以继续写任意渲染数据；Helix 只识别少量约定字段，其他字段原样保留。

Rich 图片示例：

```json
{
  "textPosition": "bottom",
  "images": [
    {
      "localPath": "/tmp/paste-a.png",
      "name": "paste-a.png",
      "contentType": "image/png",
      "size": 12345,
      "upload": {
        "status": "uploading",
        "method": "PUT",
        "url": "https://presigned-put-url",
        "headers": {},
        "objectKey": "im/2026/07/a.png"
      }
    }
  ],
  "frontendCustom": {}
}
```

File 示例：

```json
{
  "file": {
    "localPath": "/tmp/report.pdf",
    "name": "report.pdf",
    "contentType": "application/pdf",
    "size": 12345,
    "upload": {
      "status": "uploading",
      "method": "PUT",
      "url": "https://presigned-put-url",
      "headers": {},
      "objectKey": "im/2026/07/report.pdf"
    }
  }
}
```

Helix 识别字段：

- `props.textPosition`
- `props.images[]`
- `props.file`
- `localPath`
- `upload.status`
- `upload.method`
- `upload.url`
- `upload.headers`
- `upload.objectKey`

除此之外的字段 Helix 不解释，包括图片宽高、缩略图、前端布局数据、预览数据等；前端可以按渲染需要自由写入，Helix 只在更新上传状态时原样保留它们。

上传成功后，Helix 更新同一个文件节点：

```json
{
  "upload": {
    "status": "success",
    "method": "PUT",
    "objectKey": "im/2026/07/a.png",
    "url": "https://cdn-or-object-url",
    "etag": "..."
  }
}
```

上传失败后：

```json
{
  "upload": {
    "status": "failed",
    "method": "PUT",
    "objectKey": "im/2026/07/a.png",
    "error": "network timeout"
  }
}
```

## 状态模型

Post 发送状态继续使用当前四态，不扩展：

```rust
pub enum SendStatus {
    Local,
    Sending,
    Sent,
    UnSend,
}
```

本地库 `message.send_status` 继续使用当前字符串：

- `"sending"`
- `"sent"`
- `"unsend"`

文件上传状态只存在于 `props.images[i].upload.status` 或 `props.file.upload.status`：

- `"pending"`：前端传入但尚未被 Helix 接手，可选。
- `"uploading"`：Helix 已派发上传任务。
- `"success"`：该文件上传成功。
- `"failed"`：该文件上传失败。

语义边界：

- `send_status="sending"` 表示这条消息进入发送流程，不表示 `posts/create` 已经发出。
- 文件是否已上传只看 `props.*.upload.status`。
- 所有文件 `success` 后，Helix 才调用 `posts/create`。
- `posts/create` 成功或 WS echo 对账后，才把 Post 发送状态推进到 `Sent` / `"sent"`。

## 发送流程

无文件的文本消息维持现有流程。

带文件的 `rich` / `file` 消息：

1. `im_send_message` 解析并校验 `type` 与 `props`。
2. Helix 将待上传文件的 `upload.status` 统一改为 `"uploading"`。
3. Helix 本地落库同一条 `message` row：
   - `temporary_id`
   - `channel_id`
   - `message`
   - `type`
   - `props`
   - `send_status="sending"`
4. Helix 创建 `PendingSend`，保存完整 `posts/create` body 与 `remaining_uploads`。
5. Helix 为每个文件吐出一个上传 effect。
6. 单文件上传成功后，Helix 更新 `props` 中该文件节点并持久化整列 `props`。
7. 当 `remaining_uploads == 0`，Helix 使用最终 `props` 调用 `POST /posts/create`。
8. 后续 HTTP/WS 对账沿用现有发送链路。

## Effect 与 Driver 边界

现有 `Effect::Http` 不适合文件上传，因为 `HttpRequest.body` 是 `Bytes`，会把大文件读进 core/driver 之间的内存消息。

新增专用 effect：

```rust
Effect::UploadFile {
    corr: Correlation,
    req: FileUploadRequest,
}
```

`FileUploadRequest`：

```rust
pub struct FileUploadRequest {
    pub local_path: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}
```

Driver 职责：

- 从 `local_path` 读取文件。
- 使用预签名 `url`、`method`、`headers` 上传。
- 以 `PortReply` 回报成功或失败。
- 不改本地数据库。
- 不解析 `props.images` 或 `props.file`。

Helix-im 职责：

- 校验 rich/file 规则。
- 管理 `PendingSend` 与 `pending_uploads`。
- 更新 `props.*.upload.status`。
- 持久化 `message.props`。
- 判断所有上传成功后触发 `posts/create`。

## 内存结构

现有：

```rust
pending_sends: HashMap<TemporaryId, PendingSend>
```

给 `PendingSend` 增加最少字段：

```rust
body: serde_json::Value,
remaining_uploads: usize
```

新增上传 corr 路由：

```rust
pending_uploads: HashMap<Correlation, PendingUpload>
```

```rust
struct PendingUpload {
    temporary_id: TemporaryId,
    target: UploadTarget,
}

enum UploadTarget {
    RichImage { index: usize },
    File,
}
```

不新增独立 `PendingPostWithUploads` 聚合；`PendingSend` 仍是发送一致性边界。

## 上传回报处理

上传成功回包建议为 JSON：

```json
{
  "objectKey": "im/2026/07/a.png",
  "etag": "...",
  "url": "https://cdn-or-object-url"
}
```

处理：

1. 根据 `corr` 从 `pending_uploads` 找到 `temporary_id` 与目标文件位置。
2. 修改 `pending_sends[temporary_id].body.props` 中对应文件节点。
3. 写回 `message.props` 整列。
4. `remaining_uploads -= 1`。
5. 若全部成功，发 `posts/create`。

上传失败处理：

1. 修改对应文件 `upload.status="failed"`。
2. 写入 `upload.error`。
3. 写回 `message.props` 整列。
4. 不调用 `posts/create`。
5. 保留 `PendingSend`，供后续重试或取消设计使用。

本地库不需要 JSON patch；直接按 `temporary_id` 单行更新整列 `props`。

## 重试

第一版只支持失败文件重试：

```text
im_retry_upload { temporary_id, target }
```

行为：

1. 找到本地或内存中的 `props`。
2. 确认目标文件 `upload.status == "failed"`。
3. 将该文件 `upload.status` 改回 `"uploading"`。
4. 重新吐出 `Effect::UploadFile`。
5. 成功后若所有文件均为 `success`，继续 `posts/create`。

如果文件已全部成功但 `posts/create` 失败，重试发送时不重新上传文件，直接重新调用 `posts/create`。

## 取消

第一版不实现进行中上传取消。

原因：真实取消需要 driver worker、HTTP client、native/FFI 两端都支持 cancellation token，改动会扩散。

如果 UI 需要取消体验，第一版可以先做本地消息隐藏或删除；真正 `im_cancel_send` 单独设计。

## 并发与限流

Helix-im 可以一次吐出 N 个 `UploadFile` effect。

Driver 使用 `BoundedSpawner` 模式限流，例如默认 3 或 4 并发。不要在 helix-im 内部实现上传队列调度器。

## 测试计划

最小回放测试：

- `rich` + `textPosition=top` 通过。
- `rich` + `textPosition=bottom` 通过。
- `rich` 同时带 `props.file` 拒绝。
- `file` 带非空 `message` 拒绝。
- `im_send_message` 带两张图片时，产出本地 Persist 和两个 `UploadFile`，不立刻产出 `posts/create`。
- 单图上传成功后，更新 `props.images[i].upload.status="success"`。
- 全部图片上传成功后，触发 `posts/create`。
- 上传失败后，更新对应文件 `upload.status="failed"`，不触发 `posts/create`。
- `posts/create` 成功后沿用现有 `Sent` 对账。
- `posts/create` 失败或超时后沿用现有 `UnSend` 逻辑，不重传已成功上传的文件。

Driver 层测试：

- 从本地路径上传时不把文件读入 `HttpRequest.body`。
- 预签名 headers 原样透传。
- 上传成功回传 `etag/objectKey/url`。
- 上传失败回传 PortError。

## 验收标准

- `SendStatus` 枚举不新增变体。
- `message.send_status` 仍只由 Post 发送链路写入。
- 文件上传状态只写在 `props` 内。
- `helix-core` 不引入 OSS、阿里云、tokio/fs 或平台依赖。
- driver 不解析 IM props，不改数据库。
- rich/file 校验失败不产生上传 effect。
- 所有文件上传成功前不调用 `posts/create`。
