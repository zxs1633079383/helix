# 发布 Helix 并更新客户端交接
更新时间：2026-09-10 Asia/Shanghai

## 目标与授权
- 用户授权重新构建、发布公共 crates.io，并更新 cses-client 的 Helix 依赖；完成后通知。
- 默认 patch：0.1.16 → 0.1.17，待 registry plan 核实。
- 不部署、不发布桌面安装包、不自动 push；保留 artifacts/output。

## 当前身份与阶段
- Helix：reactor/MessageV3-helix，46f9cf6bb21c6fdefc9dd7f83a5abc4d7f367463。
- 客户端：tracked clean，四包当前精确 pin =0.1.16。
- 本阶段：plan通过；全量IM仅旧注册总数断言失败，已校正148并逐条断言11命令，定向7/7通过；其余全量target通过。准备publish。

## 预期
🎯 EXPECT 四包0.1.17可从crates.io解析；提交版本事务并建注释Tag；客户端四个精确pin及实际锁解析同为0.1.17且PRE cargo check通过。 | 赌注 中 | 可逆 首次上传前是

## 证据与边界
- 上阶段同源码五crate check、core测试、WASM构建、55项聚焦已通过，可复用；见上阶段交接。
- 未重跑真实Go/PRE/UI；编译不等同桌面业务验收。
- 发布前保持tracked clean，避免违反发布脚本事务白名单；SESSION在发布后同步。

## 下一步
- plan → 全量IM验证 → publish → release commit/tag → 客户端pin/check → 完成通知。

- 预检发现并修复分类接龙合并后的注册闭集断言遗漏；证据 /tmp/helix-release-0910-im-all-results.log 与 /tmp/helix-release-0910-registry-test.log。
