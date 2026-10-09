# 三仓流程图对齐交接

- 目标与授权：逐项修正 docs/mmd 漂移；仅文档，验证后本地提交，不 push。
- 状态：修正与文档验证完成；本批提交包含本文件，提交身份可由 git log -1 -- 本文件查询。
- Helix 源码基线：reactor/MessageV3-helix / 3bbce901e3d725c0cb92fd5b39e7824fd4e4245c。
- 客户端：dev-message / 46d0e13e3bc52ebd9b5b9759caca8c02d64c53bc。
- Go 审阅基线：pre-paas2.5 / c75de9aedf21eff503dbb60de54acddd6f171c5a。
- Go 收尾：86f5e38ba8dbe50e13c47152177d5631812129f5；差异仅其他任务交接文档，相关源码证据复用。
- 工作树证据：docs/mmd 的 diff SHA-256 与逐图内容指纹见 [验证结果](验证结果.json)。

## 已修正

1. 客户端迁移路径改为 packages/message 与 src/app/message-host；保留仍存在的 message.component.ts。
2. 成员快照改为 im_channels_members_by_ids → channels/member/byIds；明确双 Emit，不虚构 WS/SQLite。
3. 发送补 optimistic PersistOk、已 attach timeline 读回、HTTP 与 WS 无固定先后及 sender/receiver 出口。
4. 原始 im:* 事件与客户端 kind/action 分层；读结果明确异步事件关联，避免画成同步 PortReply 返回。
5. 书签写入删除虚构本地持久化；回复查询改为当前 im:timeline:thread 分支，更新 Go hasMore/60 上限。
6. 上传改为 Java prepare → OSS UploadFile → 本地 Complete 屏障；区分 Gate 前暂存失败与内核上传失败。
7. 分类接龙三图及总览不再误标未实现；补 mode=CATEGORY、持久事实读回与 operation/projection。
8. 补充上传失败、模板回执无上传、initial 窗口、在线态 unknown 归属、置顶查询缓存及旧发送注释的同类修正。

## 验证与范围

- 92 Gate ID/Phase/Domain 全量一致：33/20/15/12/12，无遗漏或重复。
- 客户端源码路径引用 224 处可定位；图首引用的 contract 文件存在。
- node /tmp/validate_helix_mmd.mjs：Mermaid CLI 11.3.0 的 renderMermaid 实际 SVG 渲染 94/94 PASS。
- 最终逐图 SHA-256 与验证输入一致；已确认问题的定向内容检查通过；git diff --check 通过。
- 85 张图修改；登记覆盖字段明确为合同快照，不冒充当前实测。
- 真实 HTTP/WS/SQLite/UI、多端验收均未运行；不声明全部 92 Gate 的运行链已验收。
- Java 服务端不在三仓核查范围；上传目标以 Helix 请求构造为依据。
- 原有共享脏文件未修改或暂存；生产代码、版本、依赖未改；未 push/发布。
- 后续：只有源码或合同变化、发现新偏差时补对应图与验证，不机械重跑运行链。
