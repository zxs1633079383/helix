# TooLong恢复交接
更新时间：2026-10-12 Asia/Shanghai

## 目标与边界
- 用户授权修复hydration TooLong，验证原命令完成、窗口/cursor原子提交与新游标后续请求。
- 单群5000人、每群5消息RPS、多群并行、1秒可达为目标，未做容量验收。
- Go生产源码/客户端依赖/版本未改；自动本地提交获Helix仓库授权，无新增push/发布授权。
- 上次普通G13c修复8b549a0f；10月10日3f5043b2仅红测资产，详见客户端1010-会话恢复红测交接。

## 当前代码身份
- Helix reactor/MessageV3-helix；实现提交 `b7a0ab51557f7076c609aaab1679fc2c658345f3`；文档快照另提交。
- 运行基线cc4e5cca1c9a65f6f81b52806c510e50a7ecf965，源码diff指纹043b187f3288198374aba4a4d52371c433482ae78ba5ebc69c417d27e2643105。
- 客户端dev-message基线d3348977d80f49ab93146c9bc7f842f97c21d8ab，仍pin0.1.53；原dirty保留。
- Helix原AGENTS/untracked、Go其他任务dirty均保留。版本/registry/安装包未变。

## 已完成与验证
- hydration TooLong保持pending，复用权威latest窗口；同批PersistAtomic写message+cursor后才提交/读回。
- 历史窗口不重复加未读；buffer只清已覆盖前缀；失败释放守卫/auto owner，重复/旧scope回执无效。
- 提交后经有界scheduler补拉，旧sync路由撤销，后续从新cursor请求；不禁止正常同步。
| 检查 | 命令/结果 | 证据（本目录证据/） |
|---|---|---|
| TDD红 | G13b hydration_too_long：2预期失败 | red.log |
| 定向 | G13b/G13c/gate/scope/S6：54通过；buffer unit：2通过 | focused.log、buffer.log |
| 编译/静态 | cargo check -p helix-im、定向rustfmt、diff check通过 | check.log |
| 真实HTTP/SQLite | 0→TooLong(reset6206)→原子6206→6206/no_change；再次加载6206/no_change | native-stable.json/log |
| 恢复期间新事件 | head变6207，6206/events→SQLite6207；再次加载6207/no_change | native-race.json/log |
- 每次加载唯一成功result与EventSink交付，实际读回20首屏消息；窗口提交前内存cursor不推进。
- 夹具为本机独占PG副本，206事件seq/head+6000；真实Go router，无共享Redis/Pulsar/publisher/WS。
- 本轮listener、独占PG、SQLite/wal/shm、导出及scratch已清理；共享head核对仍206。

## 阻断与未验证
- IM全套诊断停在无关G03a inbound-contract额外go字段schema失败；未改参考/断言，不称全量通过。
- 真实Tauri、在线WS、PRE/生产、1.2.75包及发布未验收；当前安装包不能声称已修复。
- 详细合同/命令/夹具/代码身份见验收记录.md与证据/代码身份.json；不把隔离夹具当线上修复。

## 下一步
- 有发布授权后确定Helix版本、按runbook发布/客户端pin，构建并做真实Tauri同场景验收。
- 有效源码指纹的定向/HTTP/SQLite结果可复用；环境或恢复代码变化只补受影响检查。
