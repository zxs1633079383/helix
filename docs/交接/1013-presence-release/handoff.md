# 在线状态按需观察发布交接

- 发布基线：origin/reactor/MessageV3-helix `b631366ea8c35c68054377f6a0694ce3690c6b0b`，四公共包 0.1.59。
- 工作分支：codex/presence-release-20261013；独立 worktree `/Users/mac17/workspace/rust/helix-presence-release-20261013`。保留原工作区及既有 artifacts，不移动原工作区 HEAD。
- 用户在已获知真实链未验收、既有全量参考合同失败之后明确授权 Helix push 和发布新版；本轮不升级客户端依赖。
- 范围：12 个 presence 源码/测试/README 文件，另含 SESSION 与本交接。命令、HTTP metadata、WS 和生命周期合同见 crate README。
- 下游要求：Go `/users/status/ids` 支持可选 `includeMetadata:true`；WS 支持有界 `presence_watch` 与完整 `presence` 快照。旧 Go 缺 metadata 时 Helix 保持 unknown，不伪造在线。
- 发布候选已验证：本工作区 presence 17、user misc 19、category 40、golden 5、event contract 3，共 84 项定向回放通过；五 crate cargo check 通过（25.88 秒）。
- 发布计划：脚本 registry 核验 0.1.59 完整可解析，0.1.60 四包未占用，PLAN PASS；准备执行发布与远端核验。

## 已有证据与缺口

原实施工作区的 presence 17 项回放、用户查询 19 项、lib 472 项、category 40 项、golden 5 项、事件合同 3 项通过，五 crate check 与 WASM build 通过。以下发布工作区验证以实际追加结果为准。

全量 `helix-im` 既有 `message_v3_mv3_g05a` 参考合同失败：实际 `im:channel:update` 字段为 channelId/id/lastPost/unreadCount/userId，参考仅 channelId/lastPost/unreadCount；未修改该域或放宽断言。真实 Go/WS/Tauri、5000 人大群及多 Pod 容量为 NOT_VERIFIED，本次发布不等于真实业务验收。

## 发布结果

- 源码提交：`0cffba91`。既有脚本 `python3.11 scripts/publish_crates.py --publish` 完成，四公共包 `cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native` 均为 **0.1.60**，registry 回读可解析（RELEASE PASS）。
- 发布脚本通过八个 workspace 包/消费者检查、四包 package dry-run/验证与按依赖顺序上传。
- 版本事务含四公共 manifest、四 workspace 消费者 manifest 与 Cargo.lock；版本提交随后建立注释 Tag `helix-v-0.1.60`，普通 push 后独立核验远端 SHA。
- 制品记录 source commit 为 `0cffba91` 加发布版本事务（发布脚本在版本提交前打包，dirty=true 为预期）。实际 registry checksum/包内源码与候选内容另行核验，未修改客户端。

## 最终核验

- 版本提交：`57b16868f763f722e6154be395104ab812c8fa21`，已普通 push 至 `reactor/MessageV3-helix`。注释 Tag `helix-v-0.1.60` 已推送，其 peeled commit 与版本提交完全一致；`git ls-remote` 独立回读确认。
- crates.io API 四包 checksum 均与本地上传 `.crate` SHA-256 一致，见 [registry-verification.json](registry-verification.json)。helix-im 包内 presence、module 接线、metadata 查询与公开事件源码逐字匹配发布候选。
- 包内 VCS：源码提交 `0cffba9176582acb61d4bc8d38f22846d79b3db2`，dirty=true 对应脚本版本事务；版本提交随后保存该事务，不把 VCS 字段伪称为最终版本提交。
- 本证据文档随后独立提交；Tag 继续指向不可变版本提交。原主工作区保持原 HEAD 与既有改动，发布 worktree 保留；客户端未升级。
