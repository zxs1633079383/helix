# 分片上传与断点续传交接
## 目标与授权
- Java/Helix/客户端普通消息Rust上传支持单文件<=1GiB，保留稳定引用与消息Persist/WS成功屏障；TDD、隔离worktree、分类提交。
- 用户授权完成后合入各自本地开发分支，由用户真实验收；本任务未push、未发布、未提版。
- >100MiB分片，<=100MiB简单PUT；8MiB/片、单文件3槽、host池最多6槽。8MiB/3槽为应用资源预算，非官方通用SDK默认值；合同见contract.md。
- 5000人/群、5消息RPS/群、多群并行、1秒可达是目标，本任务没有容量达标证据。
## 当前代码身份与合并
- Helix源验收SHA：0db52691dc1f7c7c638dc09bf77fde25cc943d93；已快进合入reactor/MessageV3-helix，后续文档提交不改变源码。
- Helix工作树：/Users/xpa-12/.codex/worktrees/multipart-1gib/helix；分支codex/multipart-1gib-20261014。
- client：86eaaa459a9c1fd2821d3889dc4e686b2f7e1036，已快进合入dev-message；工作树/System/Volumes/Data/workspace/worktrees/multipart-1gib-20261014/cses-client。
- paas-service：913075c4bf9e21d38becdbea8bfecd0b37a4ef70，已快进合入internal/v2.0.0.9-platform-5.0.3；工作树同上父目录/paas-service。
- Java CSES：dev-paas2.5基线130de35c664e5536af0592052c182288b1f5990d，工作树同上父目录/cses；继承Paas OSS实现，无新增源码提交可合并。
- CSES原仓仍为pre-paas2.5，未切换或改写PRE；各原仓其他任务dirty保留。client重叠文件已备份并三方保留，记录见证据merge-backups。
- 任务期间另一个会话的Helix频道读取修复与0.1.64版本提交已合入本地feature；本任务上传代码未发布，不能把既有0.1.64制品当新上传制品。
## 已完成行为
- Java init/parts/sign/complete/abort：认证owner隔离、durable session、OSS权威ListParts、完成元数据校验、防覆盖、完成丢响应幂等恢复。
- IM init→session PersistOk→ListParts→签名→有界range PUT→单PK ETag PersistOk→补槽→complete→稳定引用/清理PersistOk→原posts/create。
- 重试最多3次，100/200/400ms退避；失败/重试先drain旧PUT和已发checkpoint ACK，迟到响应只释放，不重复成功或写回。
- 分片总请求预算300s、源读取/等待响应空闲预算60s；不会因整文件超过30s统一失败。小文件继续原PUT路径及原预算。
- 客户端64KiB流式SHA/暂存，1GiB边界，账号4GiB/全局8GiB配额，空间预检与原子预留；sidecar同账号重启恢复、账号切换/退出清理。
- 客户端失败保留lease、立即重试可重绑定；迟到failed worker不能清掉新绑定；真实sent关联后回收。失败事件携带实际requestId，不从temporaryId猜测。
## 验证与证据
- 最终0db源码：IM send25、MV3-G01c15、G02d3、artifact schema11、IM check通过；request-id-*-final.log。
- 最终0db：host普通媒体2+分片1集成通过，五crate check通过、Web wasm build通过；final-request-id-*.log。
- core/host未变的验证复用b71及原批次：core29+1既有doc ignored、零runtime tree、range6、budget1、诊断3、URL2、shutdown1通过。
- 本地分片集成为101MiB/13片实际HTTP传输、真实Host/SQLite、模拟Java/OSS/Go/WS；1GiB边界为稀疏文件range及校验测试，不是1GiB真实OSS上传。
- Java原30单测+legacy boundary8通过；SDK adapter编译。使用/tmp init脚本的缓存javassist3.30.2替代不可取得的声明3.25，未改仓库依赖。
- CSES composite dependencyInsight及OSS编译通过；server全编译被sms/push两组Maven403阻断，未扩张修复。
- client TS12+12、package-boundaries通过；Rust stage36、command18、lifecycle1及本地Helix path cargo check通过（525既有warnings）；manifest/lock精确恢复，临时symlink已解除。
- 旧media_pipeline预合并有两次15s超时，根因未确认，随后7轮及最终合并源码通过；失败证据保留，未放宽断言，不宣称真实兼容验收。
- 热路径审查新增差异无新增对应风险；全仓audit因既有clone/JSON命中返回1。packages:status因缺archiver未执行成功，未安装/改版。
- 完整证据目录：/Users/xpa-12/.codex/worktrees/multipart-1gib/helix/.perf/multipart-20261014；Java/CSES/client摘要及最终日志已归档，部分原始日志仍在/tmp。
## 用户验收与未验证边界
- Java controller/Redis/真实OSS、真实Go/WS/Tauri、完整1GiB慢网/重启恢复、多账号实机和容量均NOT_VERIFIED，由用户验收；本地编译/模拟不替代这些证据。
- 普通im_send_message/im_retry_send覆盖本次能力；独立schedule_media/posts/createSchedule保持原简单PUT，未新增分片续传。<=100MiB失败后整文件重传。
- Web本地路径上传不在本次范围，仅协议/wasm兼容；FFI复用共享Rust但未做Flutter应用验收。公开取消Command/UI为N/A；host关闭既有有界drain/abort通过。
- 真实STS需要Multipart相关权限；OSS碎片运维生命周期配置未执行，Java保留cleanup ID而非静默TTL丢弃。未修改真实凭据/配置。
- 正常客户端pin仍使用已发布旧包；需显式本地源码入口验收新上传。原client Cargo配置存在其他任务dirty，helper会保护拒绝，使用干净task工作树。
- 验收入口：在上述client工作树运行 node scripts/with-local-helix.mjs /System/Volumes/Data/workspace/angular/visual/helix -- yarn start；退出恢复Cargo.toml/lock。
- 后续仅用户实机验收及其新授权的发布/pin更新；worktree/证据保留。本地实现和合并完成，真实验收待用户。

## 新阶段：定时消息（实施中）
- 用户新增授权：定时消息也支持分片/断点续传/<=1GiB，参数普通一致，TDD、分类提交、合入开发分支、禁止push。
- IM代理负责共享编排与durable schedule intent/公开seam；client代理负责暂存lease/定时终态与恢复；主会话负责Host集成/交接/最终合并。
- 现有createSchedule无req_id幂等wire；已发请求结果未知不盲重发，不改变Go公共合同；真实Java/OSS/Tauri验收仍单列。
- 新阶段当前：client 5d24759393f23984f481e9f217f4eb359fa66ac9已合入dev-message；38项lease回归通过，整体等待预算还在适配。
- 主会话Host新增4个共享装配场景：普通、定时101MiB成功、丢第一片回复（13次唯一PUT）、真实SQLite冷启动parts意图恢复（只PUT4..13）分别通过，尚未绑定最终IM提交身份。
- 新证据：/Users/xpa-12/.codex/worktrees/multipart-1gib/helix/.perf/schedule-multipart-20261014。host-red、host-lost-reply-red均101；first-green、lost-reply-green、cold-start通过。
- IM仍dirty实施中：schedule_media_intent、恢复、drain/retry、未知提交fence、终态清理；公开生命周期测试收敛后再固定SHA/三端验证/合并。
