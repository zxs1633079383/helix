# Hydration 终态修复交接
- 目标：修复断线/hello、复用普通 sync 失败、Persist 回灌抛错的 hydration 终态缺口。实现与本地聚焦验证完成。
- 授权：独立 worktree 实施、本地验证与完成态 commit；不改超时、不 push/发布，不操作真实业务数据。
- 仓库：/Users/xpa-12/.codex/worktrees/hydration-terminal-fix/helix
- 分支：codex/hydration-terminal-fix，无 upstream；基线 4feeab961938955f84244e21f6e3b2ea2f2a517c。
- 修复提交：以本文件所在提交为准；验证源文件 fingerprint SHA256：1f104268b9e3041ed6d7eab4261367523d0b2908fd1840e0db98e8bdecfd2960。
- 主仓仍在 reactor/MessageV3-helix，未合并本修复；worktree尚未合并故保留。合并后按用户规则保留证据再 archive，不自动删分支。

## 生命周期合同
- 入口：既有 im_channel_load_increment_by_channel_id，channel_id/channelId + transport req_id；公开 Module::handle。
- 分类：远程读，HTTP → channel/member Persist → 必要sync → 四段本地读回；写业务与新schema N/A。
- 成功：matching Persist/read-back 后唯一 im:read:result，completion=hydrated；本批保持原合同。
- 失败：HTTP/业务/解析/PortError::Timeout/Storage 错误与取消均返回关联error；unknown/非法输入保持原行为。
- 断线和有效新hello在清理前结算primary/waiter；重复hello不取消；撤销旧HTTP/read-back，保留已发出sync本地写回执。
- 普通Routine/PongGap sync的网络、解析、历史写和关闭事件写失败也结算频道hydration等待者。
- Persist业务处理Err在存在hydration等待者时转关联error并返回Ok，让EventSink可交付；无等待者保留原Err。
- 同一req_id仅结算一次，重复/迟到回执无重复终态；无新增timer/worker/transport，原30秒预算不变。

## 验证与证据
- 红测：red-1断线/hello结果缺失；red-2普通sync失败结果缺失；red-3注入Persist投影处理错误；red-4窗口/旧回包；red-5普通关闭事件PersistErr。
- 17项：cargo test -p helix-im --lib hydration -- --nocapture，PASS，含9条新回归。
- 7项：cargo test -p helix-im --lib sync::scheduler::tests，PASS。
- 35项：cargo test -p helix-im --test message_v3_mv3_g13b --test message_v3_mv3_g09d --test scope_hydration_test，PASS。
- 上述integration replay读取真实本地参考文件，环境 MESSAGE_V3_PHASE3_CLIENT_ROOT=/System/Volumes/Data/workspace/angular/visual/cses-client；首轮worktree默认相邻路径缺失，已用既有覆盖参数正确定位，未修改断言/夹具。
- cargo check -p helix-im、git diff --check：PASS；既有dead_code warnings保留。
- 热路径审查：新成功分支无消息/成员clone与I/O；取消仅冷会话边界按请求集合排序。audit脚本报告冷取消helper及原有多clone函数，人工按diff核验，不冒称脚本全绿。
- 证据目录：/Users/xpa-12/.codex/visualizations/2026/10/14/01a13994-a2cc-7112-8b81-e1521ed522a5/hydration终态修复证据
- 证据等级：公开Tick/PortReply回放；Persist处理抛错为明确内部故障注入，不是现场根因复现。真实Go/SQLite执行器/Tauri/UI、容量/1秒目标：NOT_VERIFIED。
- 无port/driver/core/schema变更，按本批聚焦边界未执行完整gate、三端Rust或无关全量测试。

## 后续边界
- 报错者安装包与本源码身份仍未核实，未证明频道1cix8ii6s7fzfjr1eoupabd9iy现场命中哪条路径。
- 后续若合并/发布/客户端升级或真实链验收，需绑定相应版本与同run证据；本批无push/发布授权。
- 规模目标仍为单群5000人、每群5RPS、多群并行、1秒可达；本批不作容量达标结论。
