# 用户级共库 / 关闭群补水终态
更新：2026-09-20 Asia/Shanghai；阶段：Helix源码修复、本地验证和远端push完成，尚未发布crate或更新PRE。

## 目标和边界
- 最新授权：关闭群补水PersistOk后读回当前公司归档投影、结束请求并清理等待；禁止恢复关闭群同步、Angular吞错/延长超时。
- 同用户外公司授权数据可补水落库；在线/离线只按当前userId+companyId/teamId投影。外公司显式读以无body错误结算，不返回渲染数据。
- 目录原样userId：helix_im/user/678/schema-v1/tenant.db；用户自行删除旧u-hex，无兼容迁移。本轮不碰真实库、运行进程、客户端/Go、版本或部署。
- increment水位仍为当前用户当前公司频道内所有作者消息max(create_at)，不是全库或本人发言。
- CSES任务6aacb31382481a450ebfd3aa保持修复中；真实PRE/UI通过前不标已修复。

## 现场证据（前阶段已核）
- PRE1.2.70安装进程确实使用user/678；不是未应用目录修改。client dev-message@3a30a4276748496295546d32cf69ca7d2175f633，当时精确依赖0.1.26。
- 2026-09-20 17:46:58.278666→17:47:28.280547，req_id=mv3-hydration-1789897618233-619efi91，corr1591。
- channel=i9zmi3pnnpbizrkq5gdr8zx8rr，user678，company=64118eebd2b665246b7880eb。
- Trace http://192.168.6.41:32281/trace/18d6fe6689d392c3：29 spans，HTTP216ms/Go15ms，HTTP200且业务SUCCESS；deleteAt=1789897616720、lastEventSeq=7、members3，needSync缺失。
- channel/member UPSERT及Persist回灌后，T5 event_effect_absent，无读回/Emit；同req_id等30001ms超时。SQLite同公司is_active0、delete_at>0、cursor7，非公司闸门误挡。
- 日志：/Users/xpa-12/Library/Application Support/com.jinqidongli.cses.pre/logs/2026-09-20/app.log，行7200/7201/7224/9462；trace含时钟偏移，不用总跨度当关键路径。
- 根因：缺needSync默认true；关闭快照标terminal；hydration仍等sync，而scheduler正确跳过terminal，导致没有后续动作。

## 实现和身份
- Helix reactor/MessageV3-helix基线90debf95f173ecb82a79e7a28630fe227e661623（0.1.27）；三个相关生产文件与registry0.1.26相同。
- 本轮提交93d798cf58d30d77e5948470790ecbffd7e5847c；三个源码/测试文件384增/2删，提交前最终diff即验证对象，提交后相关文件干净。
- 唯一业务修改：src/increment_hydration.rs，在PersistOk后用现有channel.is_terminal()走既有本地四段读回；不改scheduler/cursor/公司闸门/协议/schema。
- src/closed_hydration_tests.rs：公开Command/PortReply，当前/外公司、cursor0/7、五存储阶段失败、迟到回执、全部hydration集合/corr_map/sync窗口清理。
- host tests/user_scope_projection.rs：独占内存SQLite真实落库读回、当前公司归档终态、外公司无body；任何再次网络Effect即失败。
- 用户随后明确授权push；fetch确认远端无新增、本地仅ahead1。已非强制推送此提交到origin/reactor/MessageV3-helix，ls-remote回读完整SHA一致；未推Tag。
- 保留共享SESSION原改动和无关artifacts/交接；没有夹带未提交文件。push-only复用同SHA已有验证，仅补提交diff检查与远端核验。

## 验证证据
- 目录/tmp/helix-closed-hydration.PTrrtq/。
- red.log首次fixture类型编译错误；修正后red-runtime.log三项均在PersistOk后缺channel读回、Effect为空处失败，然后才修生产代码。
- green-lib.log：cargo test -p helix-im --lib，435/435通过，含新增3项及scheduler终态保护。
- green-sqlite.log首轮SqlValue无PartialEq编译错误；green-sqlite-final.log测试装配缺api_base_url，被入口拒绝。修正测试后sqlite-configured.log两项真实SQLite通过（0.10s），含旧公司离线重开测试。
- related.log：G13b10/scope_hydration11/render_scope13/UC452全通过；check.log：core/IM/Native/FFI/Web通过（34.34s）。git diff --check通过；未重跑Web WASM构建，本次无port/driver/依赖改动。
- hot-path-audit.log：4个原有多clone提示；本diff仅新增O(1)状态查询，无新clone/JSON/写库或平台I/O。
- 旧全IM集成4失败未扩修；历史证据见SESSION发布0.1.26/0.1.27、/tmp/helix-user-scope.aq4lFC/、/tmp/cses-userid-path.gO9RU8/account-tests.log，不冒充本轮实跑。

## 下一步和未验证
- 源码验证通过且已独立提交；SESSION/本交接保留本地，不夹带其他阶段内容。后续发布前核对该提交被包含；不重新跑未变化的同代码验证。
- 未跑真实Go/Native/UI；HTTP为现场形状fixture，真实临时SQLite不等于PRE包已通过。
- 客户端生效需另发含此提交的新Helix版本、更新Cargo.toml/Cargo.lock、重打PRE。既有0.1.27不含本修复，不得复用旧版本号上传。
- 原2026-09-18报错缺当次原始HTTP/trace，不能用本次关闭群证据倒推全部旧现象。
