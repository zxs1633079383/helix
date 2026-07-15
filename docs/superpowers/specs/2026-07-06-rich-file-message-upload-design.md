# Rich/File 消息上传发送设计

日期：2026-07-14 纠偏更新
状态：代码合同已落地；本地 HTTP 可证伪测试通过后方可称实现通过，真实对象存储仍待验收

## 安全边界

上传有两个不可混同的 URL：

- `upload.url`：短期、可能携带写权限与签名查询串的 PUT URL，只能在进程内交给 driver。
- `upload.publicUrl`：长期可读的对象/CDN URL，可进入本地 props 与 `posts/create`。

两者必须同时存在且非空。Helix 不从 PUT URL 推导 public URL，也不接受缺少
`publicUrl` 的 rich/file 发送。driver 的兼容 PortReply 仍使用 JSON 键 `url`，但该键的值
只能来自请求中的 `publicUrl`。

## 输入合同

```json
{
  "file": {
    "localPath": "/tmp/report.pdf",
    "name": "report.pdf",
    "upload": {
      "status": "pending",
      "method": "PUT",
      "url": "https://object.example/put/report.pdf?signature=short-lived",
      "publicUrl": "https://cdn.example/im/report.pdf",
      "headers": {"x-upload-token": "short-lived"},
      "objectKey": "im/report.pdf"
    }
  }
}
```

`upload.url` 与 `upload.headers` 属于写凭据。它们允许在 `PendingSend` 内存态中支撑当次
失败重试，但统一持久化入口必须剥离这两个字段。进程重启后不得从 SQLite 恢复旧签名；
后续重试需要调用方重新取得有效预签名信息。

## 类型合同

`helix-core` 用 `FileUploadUrls` 表达完整地址对：

```rust
FileUploadUrls {
    upload_url: String,
    public_url: String,
}
```

字段私有，只能通过拒绝空值的构造器创建。`FileUploadRequest` 持有该值对象；
`FileUploadResponse` 只包含 `object_key`、`public_url` 与可选 `etag`，不包含 upload URL。

## 发送流程

1. `im_send_message` 校验 rich/file 结构，同时要求 `url/publicUrl/objectKey/headers`。
2. Helix 生成 `Effect::UploadFile`；driver 只向 `upload_url` 发 PUT。
3. 乐观消息落库前，统一剥离 `upload.url` 与 `upload.headers`。
4. driver 成功后回报请求原有的 `public_url`、`object_key` 与响应 `etag`。
5. Helix 将目标节点改为 `success`，只保留 `publicUrl/objectKey/etag`，再次持久化。
6. 所有目标成功后才发 `posts/create`；其 props 不得包含 PUT URL、签名查询串或签名 headers。
7. 回报缺少 `objectKey` 或长期可读 URL 时按 malformed reply 进入 `failed`，不得发 `posts/create`。

## 可证伪验收

- 本地 HTTP server 收到的请求行必须指向带明显签名的 PUT URL。
- `FileUploadResponse.public_url` 必须等于一个明显不同的 CDN URL。
- 初次乐观持久 props、成功持久 props、最终 `posts/create` body 都不得出现签名串或 headers。
- 缺少输入 `publicUrl` 时不得产出任何 Effect。
- 上传成功回报缺少 public URL 时目标必须为 `failed`，且 `posts/create` 计数为零。

上述只证明本地合同与 driver 路由，不等同于阿里云 OSS、pre 环境或真实对象可读性验收。

## 真实阿里云前置与 Rust 缺口（2026-07-14）

本轮以现网 Java `/oss/readAccessToken` 作为外部 authority 做了脱敏活体探针：真实
`jqdl-file` 北京 endpoint 的 PUT、HEAD、GET 与 SHA-256 读回成功；探针对象已清理并确认
404。当前 `/oss/getPresignedUrl` 却按 `oss-cn-zhangjiakou` 生成 URL，真实 PUT 返回 403；
用户 STS policy 也没有 `DeleteObject`。这些是外部 authority 配置事实，不改变本仓归属。

Helix Rust 当前剩余 blocker：

1. `helix-im::send::flow` 只消费调用方预填的 `upload.url/publicUrl/objectKey`，尚未通过
   `Effect::Http` 获取 `UploadPlan`。因此 Angular/宿主仍被迫承担上传业务前置。
2. 本轮已让大写 `FILE` 与兼容输入 `file` 共用上传门控；`IMAGE/VIDEO/VOICE/AUDIO`
   的 `props.file/props.files` 目标发现与上传编排仍未接入，不能用 FILE 单测替代全部媒体验收。

正确的 Rust 收口边界：

- `helix-im` 根据媒体类型和本地文件元数据生成确定性的 plan request，发出
  `Effect::Http`，并以 corr 保存 `PendingUploadPlan`。
- plan 回包必须同时给出 `uploadUrl/publicUrl/objectKey/headers/expiresAt`；解析成功后才发
  `Effect::UploadFile`，解析失败则只落上传失败状态，禁止 `posts/create`。
- `helix-driver-host` 继续只负责 64 KiB 流式 PUT，不引入阿里云 SDK；native 与 FFI 共用。
- `cses-im-server` 是上传计划与对象确认的正式服务端入口：校验用户/频道/文件约束，调用
  阿里云 OSS 基础设施 adapter 签发计划，确认对象存在后产生可信 `MediaReference`，并继续
  承接 `/posts/create`。Java `/oss/*` 只作为迁移参考或 Go 的临时上游。
- PC Tauri 与 Flutter UI 只提交本地路径和文件元数据、绑定 Helix 投影；不得直接请求 Java、
  Go upload-plan 或 OSS，也不得自行拼最终媒体 props。
- Rust 运行配置只需要 authority URL、bucket/region/endpoint、长期读取 base URL、TTL 与大小
  上限；长期 AccessKey/Secret 不进入 Helix 配置、命令、SQLite、trace 或日志。

## 消息 props 拼装归属

Helix 必须拼最终消息 props，但不能自产可信 OSS 字段：

- UI/平台文件选择器只提供 `localPath/name/size/contentType`，视频、图片、音频可再提供
  `width/height/duration`；这些是待校验的本地元数据。
- Go `upload-complete` 返回可信 `MediaReference`：`mediaId/objectKey/publicUrl/bucket/etag/
  uploadedAt/ossType`，其中 `uploadedAt` 使用服务端时间。
- Helix 合并两侧数据，生成现网兼容的 `props.file` 或 `props.files[]`：`id` 来自
  `mediaId`，`url/uri` 来自 `publicUrl`，`lastModified` 来自 `uploadedAt`，并补齐
  `name/size/contentType/bucket/etag/ossType/userId/userName` 与媒体尺寸/时长。
- `upload.url/headers/localPath/planId` 只存在于内存上传态，最终 props、SQLite 和
  `/posts/create` 均不得携带。
- Go 在 `/posts/create` 按 `mediaId + owner` 反查并校验这些字段；Helix 负责跨端一致拼装，
  Go 负责可信性，UI 只绑定投影。

外部阿里云侧仍需把 `jqdl-file` 的 authority region 修正为 `oss-cn-beijing`，并提供服务端
cleanup role 或 E2E 前缀生命周期。Tauri/Flutter 均由 Rust native driver 直传，当前不需要
浏览器 CORS。
