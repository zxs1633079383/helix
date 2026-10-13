# 历史引用空值修复交接
## 目标与授权
- 修 Helix 无引用 null 语义，不统一其他默认值；先红测、真实 Tick/SQLite、客户端引用栏回归。
- 用户授权提交、发布新 patch、客户端依赖提交及双仓 push，不构建应用。
- 双仓同类问题与前两批客户端影响见[双仓影响审查](双仓影响审查.md)。
- 5000 人/群、5 RPS/群、多群并行、1 秒为设计目标，非容量证明。
## 代码身份与验证
- Helix reactor/MessageV3-helix：修复 513157ad；发布 0.1.61 提交 f587445cfa92ae887ded953a2550d05292226b86。
- 注释 Tag helix-v-0.1.61；core/im/host/native 四包 crates-io RELEASE PASS。
- 客户端 dev-message：回归 c08bb68676；依赖 3e0b6ebc2b655d48e21f7b43a92014885ac5fbb0，四项精确 0.1.61。
- dispatch_read 只改 repliedMessage nullable 投影；保留权威读回及其他默认值。
- 真实 Tick→生产 SQLite→读回→唯一结果事件，两导航×三输入通过；前轮 SQLite 7 项通过。
- core 测试及零运行时依赖检查通过；库 472 项、G11g/G11h 7 项；合入后 presence 17 项及三端 check/wasm 通过。
- 客户端真实组件引用栏 2 项通过；恢复原 npm 依赖后重跑通过；包边界通过。
- 四项客户端锁校验和和 locked metadata 通过，发布 IM 源码等于本地。
- 命令/代码指纹/日志见[验证记录](验证记录.md)，长日志在 .v1/历史引用修复-20261013。
## 发布与限制
- Helix 分支/Tag 已 push 且 SHA 回读一致；客户端合并远端到 a0eaf578cc97cb53a3b897eb5c5ca6a4d506180a 后 push。
- 合并保留在线状态接入和空引用防御；两 Cargo 冲突保持已发布 0.1.61。
- frozen lock 安装后定向 13 项、包边界及 locked metadata 通过。
- 客户端 release.json 原改动、双方无关 untracked 保留；check-ccc 意外升级已撤回。
- 无应用构建/安装、真实 Go/WS、Native/系统通知或容量验收。
- 全量仅三个既有失败，保持原断言及旧基线证据。
- 独立 topic={} 点击拦截已复现并记录，尚未修复，不属于引用栏生产修改。
