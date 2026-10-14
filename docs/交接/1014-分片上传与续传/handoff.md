# 分片上传与断点续传交接
## 目标与授权
- 普通、定时Rust上传单文件<=1GiB；Java/Helix/client一起兼容，TDD、分类提交、合入本地开发分支，禁止push。
- >100MiB分片，8MiB/片、文件3槽、host6槽；<=100MiB原PUT，失败整文件重试。合同见contract.md。
- 5000人/群、5消息RPS/群、多群并行、1秒可达为设计目标，未做容量达标验收。
## 当前身份与合并
- Helix源验证SHA：90dda851c80ae805087472edf4e1416f065cdeda；Host测试提交39c9ef7f；本次文档提交不改变生产源码。
- feature：codex/multipart-1gib-20261014；工作树/Users/xpa-12/.codex/worktrees/multipart-1gib/helix。
- 完成分类提交后快进到原仓reactor/MessageV3-helix；实际最终SHA用git rev-parse HEAD查看。
- client f10b1ee3ce46fec7098fae19096b6d7e04edfdc9已合入dev-message；干净worktree位于/System/Volumes/Data/workspace/worktrees/multipart-1gib-20261014/cses-client。
- Paas 913075c4bf9e21d38becdbea8bfecd0b37a4ef70已合入internal/v2.0.0.9-platform-5.0.3；本阶段复用原Java multipart接口，无新Java提交。
- CSES dev-paas2.5基线130de35c664e5536af0592052c182288b1f5990d继承Paas；原仓仍PRE，不切换或改写。
- 其他会话扫描修复及Helix0.1.65版本事务c10fd025已合入feature保留；本任务未push/发布，新定时代码不在既有0.1.65制品。
- 原仓其他任务dirty保留；验收使用干净client worktree及本地Helix路径。
## 实现
- Java认证owner、durable session、OSS权威ListParts、签名、完成校验与幂等恢复；接口init/parts/sign/complete/abort复用。
- 普通上传保持checkpoint PersistOk补槽、完成后稳定引用PersistOk再posts/create；失败/重试drain旧PUT与checkpoint ACK。
- 定时包含分片计划新增durable schedule_media_intent，上传和混合附件稳定后才原posts/createSchedule，不产生普通消息。
- 重启按owner/company恢复、权威ListParts跳过已完成片；context+提交栅栏上限32，多附件切换原子清理旧会话。
- createSchedule无幂等键，提交未知不盲重发；权威WS与持久化完成后才既有成功事件。取消只释放捕获的已提交代次，保留原流程。
- watchdog随进度旋转；分片请求300s/空闲60s，重试3次100/200/400ms。客户端大文件等待按片数预算计算，最高24h。
- client流式暂存/SHA、账号配额与1GiB边界复用；失败lease重绑定、迟到旧worker不清新绑定；纯小文件定时仍原路径。
## 验证与证据
- 90dda生产源码：IM7个定向target共116通过；five crate check、core29通过+1doc ignored、Web wasm build及core零runtime通过。
- Host4通过：普通101MiB/13片、定时13片、首片丢回复后13次唯一PUT、真实SQLite冷启动仅PUT4..13。
- 本地真实Host/SQLite和HTTP传输，Java/OSS/Go/WS均为模拟；冷启动使用持久化意图夹具，不冒充实机崩溃验收。
- client stage38、commands19通过；f10源码与Helix90dda path cargo check通过，manifest/lock精确恢复，临时symlink清除。
- Java上一阶段30单测+8边界通过；CSES OSS编译通过，完整server编译被既有sms/push Maven403阻断。
- 普通旧进度测试一次15s超时：uploaded=9、rich-video:7、pending_media complete但无稳定引用，Go未调用。
- c10 baseline初次+5复跑通过；feature临时WARN探针3次focused及完整2项通过，未复现错误；未改变断言/超时/生产代码。
- 兼容性诊断PARTIAL，怀疑既有legacy完成/调度竞态但未确认根因；失败证据保留，再现需捕获首条hop/reason并同环境对照。
- 新证据：.perf/schedule-multipart-20261014；最终日志、agent索引、client path编译及诊断摘要已归档；普通前阶段证据.perf/multipart-20261014。
## 验收与下一步
- 本地实现、分类提交与本地合并完成；真实Java/Redis/OSS/Go/WS/Tauri、完整1GiB慢网/重启、Flutter、多账号与容量NOT_VERIFIED。
- Web本地路径上传不在范围，只做共享类型/wasm兼容；无新增公开取消上传命令；真实凭据和OSS生命周期配置未修改。
- 在上述干净client worktree运行：node scripts/with-local-helix.mjs /System/Volumes/Data/workspace/angular/visual/helix -- yarn start。
- helper临时使用本地源码并恢复Cargo文件；普通registry pin仍旧包。worktree与证据保留，后续用户验收/授权发布。
