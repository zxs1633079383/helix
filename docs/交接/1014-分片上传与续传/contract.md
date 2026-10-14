# 1 GiB 分片上传冻结合同

## 范围与兼容
- 用户已授权 Java/Helix/客户端隔离 worktree 实施、TDD、分类 commit；禁止 push/发布/生产操作。
- 上限 1,073,741,824 字节，含等号；超过 104,857,600 字节（100 MiB）走 multipart；其余保持现有 PUT。
- 简单PUT中断后按原流程整文件重传；分片路径才按远端ListParts跳过已完成分片，不能把简单PUT称为按片续传。
- 阈值按用户最新指定的 ossutil 2.0 口径；[阿里云 multipart 文档](https://www.alibabacloud.com/help/en/oss/user-guide/multipart-upload)说明自动阈值为 >100 MiB。单片8 MiB符合100 KB～5 GB范围；128片是本业务1 GiB上限派生值，不是OSS的10,000片服务上限。
- 最终 fileId/bucket/uri/name/contentType/size/sha256 及 MessageV3 事件不变；合并确认及 PersistOk 前不得 posts/create。
- core 不认识 OSS/Java/IM，仅扩展文件区间传输；业务与恢复编排在 helix-im。
- 本次分片状态机覆盖普通消息 im_send_message / im_retry_send → posts/create；独立定时消息 schedule_media → posts/createSchedule 保持原PUT路径，未将其声明为新增分片续传能力。
- 当前 Web local-path 上传不支持：公共类型与 wasm 编译必须兼容，浏览器文件恢复 NOT_IMPLEMENTED_BY_SCOPE。

## Java HTTP（全部 POST，Java 成功信封沿用当前 isSuccess=true）
base=default_api_base_url，命名为 /oss/multipart/{init,parts,sign,complete,abort}。
所有请求携带 clientUploadId,bucket,contentType,size,sha256；init 可带 fileName。
除 init 外需 uploadId。Java 使用登录 userId 校验会话 owner、身份及文件规格，拒绝错误 scope。
- init：创建或恢复同一 owner+clientUploadId 会话；不同 size/hash/contentType/bucket 冲突失败。
  response={isSuccess:true,protocolVersion:1,status:"uploading"|"completed",uploadId,fileId,bucket,objectKey,uri,partSize:8388608,size,sha256}
  objectKey=uri="media/<fileId>"；fileId 稳定幂等。completed 须确认对象且规格匹配。
- parts：response={isSuccess:true,uploadId,parts:[{partNumber,size,etag}],status:"uploading"|"completed"}。
  server 查询真实 OSS ListParts 并有界分页；客户端不相信本地 checkpoint 即证明远端完成。
- sign：request 增加 partNumbers:[1..128]，每批最多8个，不得重复/越界。
  response={isSuccess:true,uploadId,expiresAt,parts:[{partNumber,url,headers}]}。
  URL 绑定唯一会话/key/part；授权仅内存使用，禁止持久化 URL/签名头/凭据。
- complete：Java 列出并校验真实分片 size/ETag/序号，合并并确认最终对象。
  response={isSuccess:true,status:"completed",uploadId,fileId,bucket,objectKey,uri,size,sha256}。
  响应丢失后可恢复；已完成查询/complete 幂等，不将 NoSuchUpload 当完成证据。
- abort：owner 校验；未完成调用 AbortMultipartUpload；重复终止幂等。
  response={isSuccess:true,status:"aborted",uploadId}；不得删除已完成对象或其他用户对象。
Java durable session（非仅进程map）记录归属、规格、uploadId、阶段与幂等；初始化外部副作用失败需恢复/补偿。

## Helix 生命周期
入口沿用 im_send_message / im_retry_send，真实类别为远程写；WS仍为发送后原有消息生命周期。
prepare→init→持久会话→parts→签名→UploadFile区间→ETag checkpoint PersistOk→缺片继续→complete→本地稳定引用原子提交→原有posts/create。
默认每文件并发<=3、全局传输<=6；可先串行正确实现后在本轮补有界并发，禁止无界task。
journal normalized：session独立行，part=(account-local session,partNumber) PK；每完成片O(1) upsert。
重试重新init/parts，跳过权威已确认片；校验重复/越界/size/etag，拒绝损坏事实。
失败沿用发送失败/UnSend，不伪造成功；PersistErr不得释放后续；重复/迟到不得重复合并/发送。
单片最大5分钟、无进度60秒；签名过期/超时先恢复对账，重试次数有界。
明确取消终止会话；关机/断网保留续传事实，清理仅run-owned资源。当前无公开取消上传入口时记录N/A并明确覆盖实际宿主关闭与worker释放。

## Driver 文件区间合同
保持 FileUploadRequest 现有字段兼容；在已有私有 FileUploadUrls 封装新增可选 FileUploadRange {offset,length} 配置。
无 range 保持整文件 PUT；有 range 校验checked_add且end<=实际文件长度，seek后take(length)，Content-Length=length。
响应ETag必须从OSS响应取得；单片进度 total_bytes=length，helix-im合并已确认字节产生现有进度。
默认整文件超时不变，range请求使用单片时限；取消/超时drop流不继续后台发送。

## 验证与交付
公开seam红测先行，记录失败原因后实现；不以只断言Effect形状替代完整PortReply/Persist回灌。
1 GiB稀疏文件可做本地范围/上限边界，真实1 GiB网络链单列不可伪称。
覆盖中断/重启、响应丢失、过期、错scope、输入冲突、缺片、重复、越界、合并响应丢失、PersistErr、取消/关闭。
三端Rust check/core test/wasm、core零runtime、受影响Java/Jest/Tauri定向检查与git diff --check。
实际Java/OSS/Go/WS环境缺失单列NOT_VERIFIED；本地替身证据为LOCAL_INTEGRATION。
5000人/群、5消息RPS/群、多群并行、消息1秒目标；上传独立预算，不等同1GiB文件1秒上传。
