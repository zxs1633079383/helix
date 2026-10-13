# 空JSON消息链修复交接
## 目标与授权
- 修复成员last_post、update_channel.lastPost、post_update.expediteMap三处空值中断链路。
- 用户授权提交、发布Helix新patch、客户端更新依赖提交、双仓push；明确不构建应用。
- 保留viewer、权威版本、读回/Persist屏障和损坏JSON校验；无虚构摘要或吞掉所有解析错误。
- 5000人/群、5RPS/群、多群并行、1秒可达是目标，非容量证据。
## 代码身份与当前阶段
- 起始Helix reactor/MessageV3-helix，0e3f1eb87ce22dd4423d29c28733ff50f3f67c70，0.1.58。
- 三处实现与回归已完成，候选patch0.1.59预检通过；当前准备修复commit及发布。
- 客户端dev-message已快进4个远端commit到f2e4b46488faea14f438e8d01a08e5c9e8957600。
- 保留客户端release.json原改动及双仓所有无关untracked；只暂存精确本任务文件。
## 证据
- 红绿、正式7项真实Tick/SQLite、十消息逐IDACK/接收唯一、负向/乱序等见[验证记录](验证记录.md)。
- 三端Rust检查、core/WASM通过；全量1690通过/3失败/11 ignored，三个范围外失败已有干净基线证据，不扩张修复。
- 源码指纹、日志在.v1/空JSON修复-20261013/；本轮无真实Go/WS/账户库操作。
## 下一步
- 修复提交后发布四公共包；立即版本commit与注释tag。
- 客户端四pin与lock更新，metadata/checksum校验后提交；双仓push并回读远端SHA/tag。
## 未验证边界
- 用户取消实机通知验收，不构建/打包/安装客户端，不改应用版本。
- PRE/Native/UI/通知中心与容量未验收；不能将13次旧错误等同八漏通知。
- 历史已推进cursor消息不会自动补发通知。
