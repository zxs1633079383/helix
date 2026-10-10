# 撤回服务端时限与结果回传

用户要求：发送满两分钟后禁止撤回，由 Go 服务端判断。

## Command 生命周期

- 入口：`im_revoke { post_id, req_id }`，由客户端 Tauri `await_query_result` 调用。
- 分类：远程写，沿用 OutboundReadReply 传输结果 correlation；wire 仍为 posts/revoke `{postId}`。
- HTTP 成功：同 req_id 回传 CommonRes；不是本地撤回成功，领域持久化/WS 路径不变。
- HTTP 业务拒绝/PortError/坏信封：现有统一失败链路返回错误，客户端显示原因。
- Persist：本次 HTTP 回传无本地写，N/A；撤回消息投影仍走既有 WS/Persist 屏障。
- 超时/取消：复用 Tauri QueryObservation 超时与代际取消；不新增 pending 或 timer。
- correlation：现有 corr_map 取出即删除；重复/迟到回包不重复 Emit。

## 证据

- focused 红测：两个撤回回传用例修复前失败。
- 基线已快进至远端 ed0366ad（0.1.50）；只同步九个 manifest/锁文件，业务源码一致。
- read_relay_test 37 项通过；registry 9 项通过；helix-im check 通过。
- 受影响两文件 rustfmt 和 diff 检查通过；整仓 fmt 存量差异不扩大修复。
- Go 定向数据库回归通过，采用本任务独立本机 cses_im_test；主 Go checkout 原有书签重复定义阻塞编译，测试使用 HEAD 加本批文件的隔离源码快照。
- 客户端 107 项定向 Jest、18 项 native command、类型/lint/包边界通过；真实桌面服务端拒绝场景尚未验证。
- 用户已授权 patch 发布；发布结果继续追加。未部署 Go 服务端，未 push。

## 发布对账

四公共crate 0.1.51均上传crates.io并可解析。版本提交d61ff5931499b534e33f902ce1cc3c4cc845375d，与注释Tag helix-v-0.1.51指向相同；源码提交2ffeffc2。客户端manifest/lock均接入0.1.51。八crate check、wasm build、core测试通过。未push。
