# 分片上传与断点续传交接
## 目标与授权
- Java/Helix/客户端支持<=1 GiB，保留业务入口及消息成功屏障；TDD、隔离worktree、分类提交，禁止push。
- 冻结合同见 contract.md；真实链与发布状态分别报告。
## 当前身份
- Helix /Users/xpa-12/.codex/worktrees/multipart-1gib/helix；codex/multipart-1gib-20261014；base e337e37e111b811144957eee4dfcdb5ae4921847。
- CSES /System/Volumes/Data/workspace/worktrees/multipart-1gib-20261014/cses；同名分支；base dev-paas2.5 130de35c66。
- paas-service 同父目录；同名分支；base internal/v2.0.0.9-platform-5.0.3 6d9717e。
- cses-client 同父目录；同名分支；base dev-message 96c6a531d7。
- 原仓dirty不复制、不暂存；主会话维护交接。
## 当前阶段
- 冻结合同与隔离完成；Java、Helix领域、客户端并行TDD；driver区间传输已实现，最终集成仍未完成。
## 已有证据
- core：29项通过、1项既有doc ignored；core wasm check通过、cargo tree零runtime。
- host区间：missing API/整文件Content-Length/缺ETag/缺超时API红测；4项区间/ETag/idle绿测。
- 上述核心/host diff指纹：be2b44c5a9ce334c48c5a52beaacf831fb5f4113a3ba93c570b2b2de675cfa30；后续新增budget/total timeout尚待统一复验。
- 证据：.perf/multipart-20261014/；仅本地证据，不代表真实OSS或消息1秒目标。
- 最新远端client f7a56c1a08与Java 9cff84f323已只读核对；保持冻结的本地开发分支快照，不扩大1914文件差异修复。
## 未验证
- 真实Java/OSS/Go/WS/Tauri、1 GiB慢网与容量，发布均未执行。
## 下一步
- Java和客户端并行TDD；主会话host区间上传，Helix领域独立租约；串行集成与分类提交。
