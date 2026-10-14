# MR9–15顺序合入交接
更新时间：2026-10-14，Asia/Shanghai

## 目标与授权
用户已批准按!9→!15合入Helix官方分支、推送远端并同步本地，继续不执行流水线。
本轮未发布crate或更新客户端依赖；5000人/群、每群5RPS、多群并行、1秒可达仍是未压测目标。

## 完成状态与代码身份
- !9至!15全部已合入reactor/MessageV3-helix，保留原提交祖先关系，未squash。
- 最终业务合并SHA：2e244758d57e062436edf227b03feff52a890fa7。
- 最终tree：d624dcd6990cbbbcf2dfc9c03740ae40ae733142，与本地组合候选完整一致。
- 本地官方分支已从bcf86c6d快进同步至最终合并提交；共享未跟踪文件保留。
- 本文后续文档提交仅更新交接与SESSION，不改变上述已验证代码树。

## 实现与兼容性
- !11保留4e399b8c追加修复：dialog专属查询先按viewer过滤；完整成员同步仍读取全部成员。
- Host SQLite与Web Session兑现ScopedScan可选filter，PC/FFI共享Host。
- !12保留WS权威确认优先、旧attempt回包清理与转发双阶段结算。
- !13导航token可选，PC继续用req_id；topic元数据不冒充父消息路由授权。
- !14 observer仅FFI接线，PC Native flush为no-op；!15仅测试夹具增强。
- !15只发生SESSION文档冲突；保留官方与MR历史，并正常合入官方祖先后完成MR合并。

## 本地验证
- 隔离目录/tmp/helix-merge-9-15的七组组合候选768a4afb：五crate check通过。
- IM同步/查询/发送/导航10个定向target共103项通过。
- 成员同步公开入口及相关lib测试15项通过，含完整roster不被viewer过滤的回归。
- Host storage_conformance 26项、too_long_reload_atomic 3项通过。
- 3项依赖外部参考文件的既有用例显式未执行；没有修改断言。
- 原!15 Rust/C++三场景证据沿用：本地WS回声、durable sent、失败与清理；未宣称真实Go/UI验收。
- 日志及每步远端tree对账：/tmp/helix-mr-merge-20261014/。

## CI与下一步
- 源分支必要空提交和所有合并提交含[skip ci]；git push附ci.skip/integrations.skip_ci。
- GitLab产生skipped空记录；回读状态和jobs见上述目录final-pipelines.json，未执行CI任务。
- 后续若交付PC/移动端产品，需要单独发布匹配crate、升级依赖并进行安装版验收。
