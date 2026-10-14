# Hydration 终态修复与发布交接
- 当前阶段：Helix开发分支合入、四包0.1.68发布、客户端依赖更新、两个远端同步完成。
- 用户授权：独立worktree实现/TDD/本地验证；后续明确授权合并开发分支、重新发布Helix、更新客户端、同步dev-message远端和dev分支。不得调大超时。
- 边界：不操作真实业务数据、不构建客户端安装包；真实Go/Tauri/UI与容量仍NOT_VERIFIED。

## 代码身份与交付
- Helix主仓：/System/Volumes/Data/workspace/angular/visual/helix，reactor/MessageV3-helix。
- 基线：4feeab961938955f84244e21f6e3b2ea2f2a517c；修复：417503b80d0384efc98b6dc4c3f9d1e8757cf119，已快进合入。
- 发布提交：8fab60f6ac9fdc21c228d5541415357a40d22c11；注释Tag helix-v-0.1.68，远端Tag解引用SHA与发布提交一致。
- crates.io四包：cses-helix-core、helix-im、helix-driver-host、helix-driver-native，均0.1.68已上传并可解析；Web/FFI/CLI/perf只同步精确依赖。
- 客户端依赖提交：72196cff5b，四pin与lock统一0.1.68；registry checksum与实际发布crate字节一致。
- 客户端最终合并：eb878063de81cf2f0ae749a9230e5874543b3bd8；origin/dev-message与origin/dev均回读为此SHA，保留并发远端d6c077cb0f更新。
- 客户端主工作区仍dev-message@8b0e7d14ff846268a52b5718bd512d8526e7155c（已含0.1.68）；本地dev为最终合并SHA。保留AGENTS.md/release.json用户脏改动；release.json与最新开发历史重叠，未为更新主checkout覆盖它。
- 原Codex修复worktree已archive，普通客户端发布worktree已删除；提交与证据均保留，不删除分支。

## 生命周期合同
- 既有远程读Command im_channel_load_increment_by_channel_id：channel_id/channelId + transport req_id，经公开Module::handle。
- 成功仍须HTTP、channel/member持久化、必要sync、四段本地读回后唯一im:read:result(completion=hydrated)。schema/ABI/30秒预算不变。
- 断线/有效新hello先失败结算primary/waiter，再撤销旧HTTP/read-back；重复hello不取消；已发出的sync本地写仍可推进cursor。
- 普通Routine/PongGap sync网络、解析、历史或关闭事件Persist失败也结算hydration；Persist处理Err有等待者时转关联error/Ok交付，无等待者保留原Err。
- 重复/迟到回包不重复终态，断线释放窗口；不新增timer/worker/transport。

## 验证与证据
- 9条新回归；三类缺口先红后绿，另含窗口/迟到回包及普通关闭事件PersistErr红测。Persist抛错为明确内部故障注入，不是现场根因复现。
- 实现指纹SHA256：1f104268b9e3041ed6d7eab4261367523d0b2908fd1840e0db98e8bdecfd2960；发布仅manifest/lock变动，源文件身份未变。
- 59项定向：17 hydration库回放、7 scheduler、35 G13b/G09d/scope回放通过；参考根MESSAGE_V3_PHASE3_CLIENT_ROOT指向真实本地cses-client文件。
- 发布前cargo test -p helix-im --lib：490/490通过；core测试28项及doc test1项通过、文档示例ignore1项；Web wasm32 build通过。
- 发布脚本八workspace crate check、四包dry-run/发布解析通过；diff检查通过。既有dead_code warnings保留。
- 客户端cargo update仅四包、cargo tree --locked解析正确；最终合并后3组Jest 27/27通过（Hydration/Engagement/channel-authority-chain）。
- 热路径人工diff审查：成功分支无新增消息/成员复制或I/O；冷会话取消按请求排序。audit脚本报告冷helper及原有clone，不冒称脚本全绿。
- 证据：/Users/xpa-12/.codex/visualizations/2026/10/14/01a13994-a2cc-7112-8b81-e1521ed522a5/hydration终态修复证据
- 分层：公开Tick回放、Rust编译、registry发布/依赖解析、Jest均通过；真实Go/SQLite执行器/Tauri/UI、客户端打包/安装及CI/CD结果未验证。
- 规模目标：单群5000人、每群5RPS、多群并行、1秒可达；本批不作达标结论。

## 后续
- 报错者安装包版本仍未核实；尚未证明频道1cix8ii6s7fzfjr1eoupabd9iy现场命中哪条路径。
- 客户端主checkout更新到最终远端前须协调release.json未提交编辑；保持用户内容，不能直接覆盖或强制重置。
