# 成员空列表修复交接
更新时间：2026-10-13，Asia/Shanghai

## 目标与边界
- 空字符串列表中断post读回、ACK/接收事件的修复与发布、客户端依赖接入、双仓push已完成。
- 用户取消PRE十消息/通知中心验收，要求真实Tick验证；后续授权打包发布与双仓push，再明确客户端只改Helix依赖直接提交，不构建应用。
- 严格非空损坏数据校验、viewer校验、Persist屏障保留。5000人/群、5RPS/群、多群并行、1秒可达为目标，非实测结论。

## 当前代码身份
- Helix：`reactor/MessageV3-helix`；修复`52cda2a65159e478b452c8793aa8490b190f8dd2`。
- 四公共包0.1.58发布commit：`f2bef81aac7eda2c9abe220f15b5313202b98865`；注释Tag `helix-v-0.1.58`已push，远端peel到同一发布commit。
- 客户端：`/System/Volumes/Data/workspace/angular/visual/cses-client`，`dev-message`；依赖接入`8f69f8085763db6882714e7bdc446374d8e1f556`，已push且远端回读一致。
- 客户端先快进同步11个已有远端commit；本次只提交src-tauri/Cargo.toml与Cargo.lock。原有release.json本地修改、所有初始untracked保留。
- Helix最终仅补交接文档，最近文档提交身份见git log；无关初始untracked未暂存。

## 已完成与验证
- 原始EOF、迁移和空写红测；G01 4项、真实SQLite 2项通过，十消息逐ID持久化/ACK/接收/频道更新/ACK终态唯一。
- helix-im库472项、五crate check、core测试、WASM build、core零runtime、rustfmt/diff check通过。两项扩大回归失败在干净起始SHA复现，见[验证记录](验证记录.md)。
- `python3 scripts/publish_crates.py --publish`：八crate check、四包dry-run/上传/registry可解析均通过。
- registry下载helix-im制品的六个修复源文件与本地字节一致。
- 客户端四个精确pin/lock均0.1.58，四包registry制品checksum逐包一致；cargo metadata --locked --no-deps和diff check通过；未构建应用。
- 双仓分支/版本Tag push成功且真实远端SHA回读一致；发布提交与注释Tag完整。
- 完整本地日志、制品校验、发布接入结果：`.v1/成员空列表-20261013/`，不随Git上传。

## 未验证与边界
- 已安装应用不会仅因Git依赖更新而改变；本轮未编译/打包/安装客户端，未更新应用版本。
- 真实Go/WS、Native/Tauri/Angular、系统通知与容量未验收；不将Tick/SQLite或crate发布晋级为运行态验收。
- 错误次数不等于漏通知条数；历史已推进cursor的消息不会自动补发通知。
- 旧默认兼容使用PK触发器，不重建既有SQLite表；启动扫描与逐消息复杂度分开。

## 下一步
- 本轮授权范围内无剩余操作；应用构建/安装及通知逐ID验收按用户后续要求另行执行。
- 同代码/依赖/环境证据复用；既有全量失败属于独立修复范围。
