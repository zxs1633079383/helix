# 可见消息回应查询

## 范围与合同

当前分支 `reactor/MessageV3-helix`，当前工作区实现；用户仅授权实现验证，不提交、不发布、不提版。

- 远程读入口 `im_reaction_unread` 增加可选 `post_ids: string[]`，HTTP `posts/reactionUnread` 映射 `postIds`。最多100个不带首尾空白、非空、不重复ID；显式空数组保留，缺省省略。
- Go 响应仍含 `count/nextTarget/memberProjection`，可选 `targets: {postId,revision}[]` 逐项验证字符串版本、ID、去重及100上限，不得超过count。旧响应缺省targets仍接受。
- 原ACK命令与接口不变；不占用群seq、不增加本地schema、driver或core协议。
- 成功仍经HTTP → 原子持久投影 → matching读回 → `im:channel:update` / 关联`im:read:result`。过滤仅影响返回的targets，不裁剪频道全局count。
- 非法输入无HTTP；HTTP/业务错误及非法响应不持久化；存储失败/投影冲突给关联错误。超时、取消、重复/迟到响应沿用现有corr清理。查询无服务端写入；ack之外远端写生命周期N/A。

## 验证

证据：`artifacts/visible-reaction-query-20261014/`。

- G13新增3个公开Tick回归先红后绿，整个target 14/14通过：空/部分/100可见ID过滤、精确大版本字符串、持久屏障；非法过滤/响应；原query/ack成功、失败、取消及迟到回复。
- `cargo check -p helix-im`、五crate native/FFI/Web检查通过。
- WASM `helix-driver-web`构建通过。
- core 29 passed，1既有doc ignored。
- 真实Go/Tauri业务验证由主任务继续；此证据不证明真实桌面自动清除。

- 全量 `cargo test -p helix-im` 非绿：`message_v3_artifact_schema` 六项依赖旧绝对路径 `/System/Volumes/Data/workspace/angular/visual/cses-client/specs/...`，文件缺失；该合同未被本次修改。不扩大修复范围，后续target并未全部执行。
