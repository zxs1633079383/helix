# 全局书签查询

用户允许跨 Go、Helix、客户端修改；代码范围，不改版本、不发布、不 push。

## 合同与生命周期

- 新增只读 `im_bookmark_load_global`，payload：`page_number` (1..1000000)、`page_size` (1..100)、可选内部 `req_id`；拒绝 actor/company/channel/offset 输入。Native Host 始终提供 req_id。
- HTTP POST `/post/bookmark/global`，body 仅 `pageNumber/pageSize`。认证用户和企业由现有会话 Header 提供。
- Go 成功 `data={userId,companyId,data:Post[],hasMore}`；当前用户仍为成员、同企业、消息 viewers 可见，收藏时间及 ID 倒序，分页前过滤。
- Helix 检查响应 actor/company 等于当前会话，复用消息摘要投影。未缓存频道仅得到该 req_id 本轮渲染许可，发出结果即消耗；不改变持久频道归属，也不放宽群内读取。
- 成功 `im:read:result {req_id,body}`；输入非法 fail closed；HTTP/网络/超时/坏响应/业务失败/外企业结果进入 error。identity 变更及停止取消普通读关联，重复/迟到 PortReply 不再次 Emit。
- 远程只读，不写缓存：Persist/PersistErr N/A；HTTP 传输超时由现有驱动回灌；没有新增 timer。原生 waiter 超时、窗口关闭与 runtime generation 拒绝由既有 Host 生命周期负责。

## 验证

- 红测：新增公开命令尚未实现时没有 Http，生命周期断言失败。
- 实现：`read_relay_test`、`p3a_posts_read_test`、`render_scope_test` 共 75 项通过。包含未缓存群、错误公司、失败、重复、单次授权消耗、超时、停止。
- 五 crate check、core 测试和 wasm build 已通过；最终版本五 crate check、wasm build 通过。完整 helix-im 套件在 `message_v3_artifact_schema` 因历史绝对路径 `/System/Volumes/Data/workspace/angular/visual/cses-client/specs/.../inbound-contract.json` 不存在失败（5 失败），不修改历史 fixture 路由绕过。结果看 `/tmp/global-bookmarks-*.log`。
- 生命周期红测发现共用 stop 未取消普通读，已复用身份变更的取消逻辑修复；未改其他写路径。
- 真实 Go/native/UI 同轮未验证；本机隔离 PostgreSQL 55432 拒绝连接，客户端仍固定 registry Helix 0.1.43，不能声称新命令已在安装版本运行。
- 不新增 harness card；保持 HX-C018 领域投影与租户围栏，变更只作用于该远端读的单次已认证结果。

- 提交前 codebase-memory detect_changes 基于 HEAD：64 seed、14 impacted，主要为读回调与既有渲染测试；工作区包含无关 untracked artifacts，按精确文件清单排除。
