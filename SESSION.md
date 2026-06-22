# Helix SESSION

> 跨会话状态可查。§0 记三类固定前缀行（均可 grep · 详见 CLAUDE.md §4.8）：
> 开局 `🎯 EXPECT`（实质任务预判）· 收尾 `✅ DONE`（终态）· 偏离预期时 `⚡ DRIFT-PRED`（喂 §4.7 drift-review）。

## §0 终态记录

✅ DONE P1.5 出站横切层迁移（鉴权/401/断网/trace）@2026-06-22 19:40 +0800 | commit 本提交 | 分支 feat/im-takeover
  ↳ 落地：① core 新 `auth.rs`（generic 鉴权意图契约：`AUTH_KIND_HEADER="X-Auth-Kind"` + `AuthKind{Session,Bot}` as_str/from_str；core 标意图不拼 token）；② host 新 `http_cross.rs`（`CrossCuttingHttp<H,E>` decorator over HttpRequester + `AuthTokenRegistry` 双槽 session/bot：读意图头→注真 token→strip 意图头；traceparent 原样透传；401→emit `im:http:unauthorized`、`PortError::Transport`(断网)→emit `im:net:offline` 且错误原样上抛）；③ native `http.rs` 加 `NativeHttp::with_crosscut`（装配横切层 + 保留 registry 句柄 → 通用头注入路径不受影响，删旧 `unreachable!`）；④ 新 `tests/p1_5_http_crosscut_test.rs`(host, 13 用例：session/bot 注入+strip / traceparent 透传==入站 / 401·断网 emit / 403·http-err 不误 emit / 意图缺失·未知值·无 token 不注入 / 同名头顶替 / clear 独立)。
  ↳ 验证：cargo test host(13 crosscut+既有) / native(5 http 含 crosscut smoke) / core(3 auth) / im(48+ 全绿) 全过；cargo check 四 crate 过；core 零运行时 grep 空；C6 im/src 无真 token（grep `mp_` 全是 bump_/tmp_ 假阳，无 `Bearer mp_`/`Authorization` 字面量 → token 只在 host http_cross.rs）；C3 im 无 ambient time；`auth.rs` 覆盖 100%(22/22 行·5/5 函数)、`http_cross.rs` 100%(56/56 行·13/13 函数·81/81 region)；structure-gate PASS（新文件 auth 75/http_cross 182 ≤300）。
  ↳ ⚡ DRIFT-PRED 横切层落点偏离 plan：02/05 文档写 native-only `http_manager.rs(Arc<RwLock<HeaderMap>>)`，实际下沉 **helix-driver-host** `http_cross.rs`（ADR-007 双端共享：PC native + Flutter FFI 共用一份出站鉴权/错误语义，避免两端漂移；host `network.rs` 已有 `HostHeaderRegistry` 同款 `Arc<RwLock<HeaderMap>>`，无需 native 重造）→ 候选教训：「driver 横切层凡 PC+FFI 共享语义的，落 host 不落 native；plan 写 native-only 是 rv-tauri 视角未下沉 ADR-007」。plan 文档 P1.5 实现文件名/落点待 main 对齐（架构更对，非回退）。

🎯 EXPECT P1.5 出站横切层迁移（鉴权/401/断网/trace）@2026-06-22 19:25 +0800 | 赌注 中 | 可逆 是
  ↳ 可证伪预判：① 横切层落在 **helix-driver-host**（ADR-007 双端共享，非 plan 写的 native-only `http_manager.rs`）新增 `http_cross.rs`：`CrossCuttingHttp<H,E>` decorator 实现 `HttpRequester`，包内层 HttpRequester + EventSink + 鉴权 token 源（`AuthTokenRegistry`）；native `http.rs` 加 `NativeHttp::with_crosscut` 装配点；② core 侧仅约定 `X-Auth-Kind: bot|session` 意图头常量（放 `helix-core/src/effect.rs` 或新 `auth.rs`），**不拼 token**——decorator 读意图头→注真 token→strip 意图头；③ 401 响应→emit `{"event":"im:http:unauthorized","data":{...}}`、`PortError::Transport`(断网)→emit `{"event":"im:net:offline",...}`，两者经 `to_bus_envelope` channel 提取正确；④ traceparent 出站头透传：req.headers 里的 traceparent 原样到 prepared 出站头（断言出站==入站信封 trace）；⑤ 新 `tests/p1_5_http_crosscut_test.rs`(driver-host) 全过：401 emit / 断网 emit / traceparent 透传 / 鉴权意图→token 注入 strip / 200 不误 emit；⑥ `grep -rE 'mp_|Authorization|Bearer' crates/helix-im/src` 必空(C6)、`grep` core 无 token；⑦ `cargo test -p helix-driver-host -p helix-driver-native`、`cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` 过、`cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 空；⑧ `http_cross.rs` 行/函数覆盖 100%。BLOCKER 候选：plan 写 native `http_manager.rs(Arc<RwLock<HeaderMap>>)`，但实存 host `network.rs` 已有 `HostHeaderRegistry(Arc<RwLock<HeaderMap>>)` + 共享 client → 横切层下沉 host 复用既有 registry 更对（双端共享，不在 native 重造）；此偏离记 DRIFT-PRED 留 main 裁决文档对齐。

✅ DONE P1 SendPosts 编排下沉（接管串行根）@2026-06-22 +0800 | commit 本提交 | 分支 feat/im-takeover
  ↳ 落地：新 `outbound/send_build.rs`（buildMessageObject 纯函数 + build_from_command 命令解析 + optimistic_persist_op；viewers 空补['all']/findReplyRootId 三分支/userSnapshot 5 字段/2000 截断/空消息 trim 拦截→None）；`module.rs::handle_send_message` 改接收 now_ms（Clock 注入 createAt 非 createdAt）+ 调 send_build（body 全 camelCase 对齐真抓包 §1）；`pending_send.rs` PendingSend 加 connection_id 维度（M1）；ImConfig 加 user_name/org_name/dept_name（userSnapshot 占位）。新 `tests/p1_send_build_test.rs`（20 用例：纯函数全分支 + 端到端 body/对账 + posted vs post echo 锚 + 15s timer）。
  ↳ 验证：cargo test -p helix-im 全绿（31 bin）；cargo check 四 crate 过；core 零运行时/C3/C6 grep 全空；send_build.rs 覆盖 100% lines+functions+regions；structure-gate PASS（module.rs 收口回 1210≤1211 基线，Persist 抽到 send_build）。
  ↳ ⚡ DRIFT-PRED 候选教训：开局预判 fail-hard①「喂 posted 帧 reconcile 不命中」无法在 P1 直接成立——helix-im 现存 legacy `posted` handler 仍按 temporary_id reconcile（与真源「send echo 只走 post」冲突）；删它属 P4 dead-action 清理且会破 echo_reconcile_test，已改为锚定 `post` 是真路径并把矛盾留 main 裁决。

🎯 EXPECT P1 SendPosts 编排下沉（接管串行根）@2026-06-22 +0800 | 赌注 高 | 可逆 是
  ↳ 可证伪预判：① 新增 `crates/helix-im/src/outbound/send_build.rs`（buildMessageObject 纯函数：viewers 空补['all']、findReplyRootId 三分支、userSnapshot 5 字段组装、文本 2000 截断、空消息 trim 返回 None），`module.rs::handle_send_message` 改为调用它且改为接收 `now_ms` 注入 createAt；② posts/create body 含 `createAt` 不含 `createdAt`、含 `viewers/type:"TEXT"/simpleMessage/userId/teamId/userSnapshot/mentions/props/topicId/revoke/id`、回声对账锚 `action=="post"` 非 `posted`、temporaryId 主键全程不变；③ `grep -rE 'SystemTime|Instant::now|Utc::now' crates/helix-im/src` 必空（C3）；④ 新 `tests/p1_send_build_test.rs` 全过 + 既有 `send_test/echo_reconcile_test/s5_post_echo_test` 仍绿；⑤ `cargo test -p helix-im` 全绿、`cargo check -p helix-core -p helix-im -p helix-driver-native` 过、`cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 空；⑥ `send_build.rs` 行/函数覆盖 100%。BLOCKER 候选：userSnapshot 的 orgName/deptName/userName 是 ucenter 跨 feature 读依赖，本 Phase 用 ImConfig 占位字段（空串 mock），不阻塞自身。

✅ DONE **全树 .md 熵管理 + 进度同步**（2026-06-22 · 分支 `docs/entropy-2026-06-22`，未提交待审）| 4 read-only triage agent 扫 126 篇 .md + expert-panel-review 全树架构评审佐证 | 改 ~22 篇：① 落地真相 `crates/helix-im/docs/sync-v2-能力与缺口状态.md` 大改（cursor 真源 `_helix_monotonic`→`channel_event_cursor` 纠偏[与 arch-review #1 HIGH 同源] · §2/§3/§5 send-echo/sync-风暴/parser-post 全标已落地入 main · 删"未提交"假象）② 根 `README.md` 三栖→两端(ADR-006 wasm 归档)+修死链 ③ `ROADMAP.md` E9 拆 E9a✅/E9b open + cursor 注脚 ④ wasm `CLAUDE/AGENTS` 加归档 banner(镜像同步) ⑤ ffi design README F4 未接→已落地 ⑥ 11 篇历史稿(Bun-checklist/ws-registry-plan/merge-spec/sync-v2-report audit×5/迁移 01·02/下游域卡)加归档 banner ⑦ refactor A/B 状态标注(B 已落地·A 部分) ⑧ 修 9 条死链(eventbus/sync-v2 合并稿) ⑨ docs/README 对账日期 + HX-C 计数 C009→C012 | 验收：7 crate CLAUDE/AGENTS 镜像全 same · harness 索引 12 · structure-gate/gate.sh 全 PASS · 死链 0 | ⚠️ 另：`docs/entropy-sync` 死分支(merge-base 06-12·落后 main 250 commit·工作被 4a3edb8 取代)建议删；arch-review 另出 4 条代码 finding(timer_map 泄漏 HIGH 等)未在本轮改(属代码非文档)


🎯 EXPECT Task4 P2/P3 audit + gate alignment @2026-06-22 16:28 +0800 | 赌注 中 | 可逆 是
  ↳ 可证伪预判：① 固定 `ED380414...` deviceId 只作为 host-cli dev-bin 默认兜底 + ADR/docs/历史联调证据出现，不进入 native/FFI/Tauri delivery 代码路径；② `scripts/gate.sh` 明确默认覆盖 PC native + Flutter FFI 仓内验证，并调用 `scripts/ffi-verify.sh --no-demo`；③ wasm 保持 ADR-006 archived/manual，不再作为默认交付 gate 必跑项；④ `cargo test -p helix-im --no-run` 无 `Harness::new` dead_code warning；⑤ `cargo test -p helix-im --no-run`、`scripts/ffi-verify.sh --no-demo`、`scripts/gate.sh` 结果可记录，GitNexus `detect_changes` 可运行或明确不可用。
✅ DONE Task4 P2/P3 audit + gate alignment @2026-06-22 16:48 +0800 | commit 本提交
  ↳ 落地：deviceId 精确 audit 确认固定真值只在 `crates/helix-host-cli/src/main.rs` 作为 ADR-011 dev-bin 兜底；delivery crate 未引入该硬编码。新增 ADR-012，把默认 gate 明确为 PC native + Flutter FFI，`scripts/gate.sh` 纳入 `scripts/ffi-verify.sh --no-demo`，wasm 维持 ADR-006 archived/manual；`scripts/ffi-verify.sh` 同步说明默认 gate 与 demo/full 边界。`Harness::new` 已有窄 `#[allow(dead_code)]`，no-run 无 warning。
  ↳ 验证：`cargo test -p helix-im --no-run` PASS；`scripts/ffi-verify.sh --no-demo` PASS（既有 core/im clippy warnings 非硬失败）；`scripts/gate.sh` PASS（cargo-public-api 未安装按既有逻辑跳过）；GitNexus `detect_changes(scope=staged)` low risk / 0 affected processes。

🎯 EXPECT Task3 StorageOp::BatchDelete 跨端覆盖与 public-port 文档收口 @2026-06-22 16:00 +0800 | 赌注 中 | 可逆 是
  ↳ 可证伪预判：① 只改 Task3 范围内 storage/core/driver/docs/report 文件（不碰 .idea/AGENTS/CLAUDE/helix-im fixture/Task1-2 范围）；② host 真实 SQLite BatchDelete 覆盖已存在或只补真实缺口，native/ffi 通过 shared host-driver 边界有明确测试/文档证据；③ wasm 只保持 archived/experimental trait-shape，不进入交付/CI；④ `cargo test -p helix-core -p helix-driver-host -p helix-driver-native -p helix-driver-ffi`、`cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` 通过，`cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 空；⑤ GitNexus `detect_changes` 可运行并显示改动范围符合预期。
✅ DONE Task3 StorageOp::BatchDelete 跨端覆盖与 public-port 文档收口 @2026-06-22 16:55 +0800 | commit 本提交
  ↳ 落地：补 host `execute_storage_ops` 分发 `StorageOp::BatchDelete` 真 SQLite 回归锚；确认 native/FFI 均复用 shared `HostStorage`，并把 storage port、host/core/ffi 设计图、ADR/ROADMAP/sync-v2 文档更新到 PC Tauri + Flutter FFI shared host-driver 事实；wasm storage 注明 archived/experimental trait-shape only；清理 BatchDelete 已接通后的 stale 注释；修正 FFI/C++ smoke 里因 Task1 严格 ID 校验暴露的旧大写 channel_id fixture。
  ↳ 验证：GitNexus context(batch_delete/StorageOp) 已跑，`detect_changes(scope=all)` 可运行（全工作树含用户既有 AGENTS/CLAUDE dirty，报 high）；`cargo test -p helix-core -p helix-driver-host -p helix-driver-native -p helix-driver-ffi` PASS；`cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` PASS；`cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 无输出/exit 1；`scripts/cpp-smoke.sh` PASS；`git diff --check` PASS。

🎯 EXPECT 批次A修复 review P1-ID API 与 ID 校验 @2026-06-22 | 赌注 高 | 可逆 是
  ↳ 可证伪预判：① 生产公开 API 不再暴露 `Id26::from_test_u64` / `ChannelId::from_test_u64` / `ServerId::from_test_u64`；② `Id26::from_str` 只接受 26 位 `[a-z0-9]`，拒绝空格/标点/大写/下划线；③ `s6_increment_end_fanout_deterministic_order` 改用固定真实形态 26 字符 fixture，不再依赖零填充数字的字典序；④ `cargo test -p helix-im` 通过；⑤ 本批不触碰 whitespace/gate/BatchDelete 实现。
✅ DONE 批次A修复 review P1-ID API 与 ID 校验 @2026-06-22
  ↳ 落地：生产公开 API 删除 `Id26/ChannelId/ServerId::from_test_u64`；`Id26::from_str` 收紧为 26 位 `[a-z0-9]`；helix-im/driver-host/driver-ffi 测试与 bench 改用固定真实形态 26 字符 fixture；S6 fan-out 排序测试改用非数值序 fixture；`Harness::new` dead_code warning 清理。
  ↳ 验证：RED `cargo test -p helix-im id26_tests::from_str_rejects_invalid_ascii_alphabet` 先失败（旧实现接受 `!`）→ 修复后 PASS；`cargo test -p helix-im` PASS；`cargo test -p helix-driver-host --test im_event_persist` PASS；`cargo test -p helix-driver-ffi --test f5_egress_parity` PASS；`cargo check -p helix-im --benches` PASS；代码/测试/bench 范围 `rg from_test_u64` 0 命中；工作树级 `git diff --check` PASS。`fa07ff1..HEAD` 历史 docs whitespace 仍留批次B。
🧭✅ **新会话开局先读这条（2026-06-22 · 本轮全收口）**：本会话全部工作已**合并到 `main`**（主仓 `/Users/mac28/workspace/rustWorkspace/helix`，已 push `origin/main`；临时分支 `feat/sync-v2-followup` + 旧 `dev-zlc` 已删、worktree 已移除）。**新会话在主仓 `main` 分支上续**。sync-v2 wire 对齐 + 真机 e2e 8/8 + drift-review 回正 + plan-gate 约定 + **#4 FFI pump 锁跨 await 彻底解** + 合并推送 **全部落地·无开放项**。
  ✅ DONE **#4 FFI pump 锁跨 connect().await 彻底解**（commit `34c54bd`）：注册通道——连接期 transport 独占 loop 外后台跑 connect(裸 ws·不在表)→连成功经 channel 投进 loop 私有表→连接期 transports.get()=None 走 warn（**类型层不可见**·不靠时序不变量）。Transport send/recv/close 收紧 `&self`(connect 保 &mut)、TransportTable 去外层 Mutex、不新增 per-Send 锁、create 非阻塞不回退。验：5-crate+wasm core · driver 全测绿(**lifecycle_net_decouple 3/3·create/destroy 不阻塞**) · 零运行时 · gate+structure-gate。
  ⚠️ **落地坑沉淀（实测·别再踩）**：#4 设计阶段「自动落地反复失败」是**假象**——`Workflow({name})` 工具的 agent 与 fork **实际编辑落在主仓 git root（`/Users/mac28/workspace/rustWorkspace/helix`），不落当前 worktree**；我一直在 worktree 查 status 才误判「丢失」。**教训：workflow/fork agent 文件改动落主仓、不落 worktree——在主仓查 status/commit。**（下方 📋 SPEC 保留作 #4 实现的设计参考。）
  ⚠️ **落地坑（实测·别再踩）**：#4 **3 次自动尝试全失败落地**——2 次 `Workflow({name})` 工具的 agent **跑隔离副本、编辑不落主工作树**；1 次 fork 空跑 34s。**教训：用主对话自己的 Edit/Write 落地（实测持久——本会话 plan-gate `534b524` / 回正 `615f2f3..` 都是这么落的），或盯紧的 fork（回正那个成功了）；落完即 commit 锁定**（未提交改动会被某 rustfmt-restore 链路吃掉，那 3 个反复出现的噪音文件 engine.rs/spawner.rs/lifecycle 测就是诱因，别整树 git restore）。当前 #4 仅「in-code 文档化(commit 2a24df7) + ROADMAP-P」interim，**合法非阻塞**。

  ## 📋 SPEC — #4 FFI pump 锁跨 await（已验证·照此实现·无需重新设计）
  - **objective**：消除 FFI pump 后台连接任务持 `Arc<Mutex>` 跨 `connect().await`(~30s) 的锁跨 await 脆弱点（现靠 hello-gated 时序不变量、非类型强制）。彻底解 = 连接期句柄**类型层不可见**。
  - **approach（注册通道·主干取方案A）**：连接期 transport 独占在 loop 外后台 spawn 任务跑 connect（**裸 ws·无 Arc/Mutex·不在表**）→ 连成功经 `transport_tx` channel 投进 loop 私有 `mut transports` 表 → 连接期 `transports.get()=None` 走 dispatch warn 兜底（类型层不可见，不靠时序）。表内 `Arc<Tr>`（无外层 Mutex）；send 走 `&self`；并发由 `SharedWsClient` 既有内部 `state: Arc<Mutex<WsState>>` 串行化（**非新增 per-Send 锁**）。**connect 保 `&mut self`**——NativeTransport 状态机字段 `connected/disconnected_emitted` 真改、合法需要（方案B「四方法全 &self」被此否决）。
  - **filesToChange（9）**：
    1. `helix-core/src/ports/transport.rs`：`send/recv/close` `&mut self`→`&self`（connect 不动）。唯一 core 改 = 签名收紧，非 Effect/Tick 变体。
    2. `helix-driver-host/src/network.rs`：SharedWsClient `send/recv/close` 去 `&mut`（零逻辑改，内部已 state.lock()）。
    3. `helix-driver-host/src/engine.rs`：`TransportTable<Tr>` 去外层 Mutex→`HashMap<TransportId,Arc<Tr>>`；`run_engine_loop` +`mut transports` +`transport_rx: mpsc::UnboundedReceiver<(TransportId,Arc<Tr>)>`；**biased select 第四臂**(紧随 reply_rx 后)：`Some((id,tr))=transport_rx.recv()=>{transports.insert(id,tr);continue;}`；Tr bound +`Sync`；改写头注②死锁分析为类型层论证。
    4. `helix-driver-host/src/dispatch.rs`：Send 臂 `t.lock().await.send(frame).await`→`t.send(frame).await`。
    5. `helix-driver-host/src/lib.rs`：`run_engine_loop_with_transports` +`transport_rx` 透传+re-export。
    6. `helix-driver-native/src/transport.rs`：NativeTransport `send/recv/close`→`&self`；`inner_mut` 拆(connect 用 / send 用新增 `inner(&self)`)；`connected/disconnected_emitted` `bool`→`AtomicBool`；`emit_disconnected_once`→`&self`；connect 保 `&mut`。
    7. `helix-driver-native/src/engine_loop.rs`：TransportTable 去 Mutex；内部构造空 `transport_rx` 透传(native 走预填表·channel 永空)。
    8. `helix-driver-ffi/src/pump.rs`：删 `ws_handle=Arc::new(Mutex::new(ws))`+旧 insert；建 `(transport_tx,transport_rx)=mpsc::unbounded_channel()`；transports 起空表；后台任务 `let mut ws=ws` move 跑 connect 成功后 `transport_tx.send((main_transport,Arc::new(ws)))`；run_engine_loop +transport_rx；**ready_tx 仍 spawn 前立即回传**(create 非阻塞不回退)；删 108-136 时序长注释换类型层短注。
    9. `helix-driver-wasm/src/transport.rs`（archived）：`send/recv/close`→`&self` 保编译一致。
  - **acceptance**：`cargo check` 5-crate + `--target wasm32 -p helix-core`；`cargo test` driver-host/ffi/native（**lifecycle_net_decouple 绿**·create<2s/destroy<4s vs blackhole 192.0.2.1）；`cargo tree -p helix-core` grep tokio/sqlx/.. 空；`gate.sh`+`structure-gate` 绿。类型层证据 = grep 确认 dispatch/engine 无 transport `.lock()`、连接期句柄不在表。
  - **boundaries**：connect 必保 `&mut`；不新增 per-Send 锁；不回退 create 非阻塞；core 仅签名收紧。
  - **openQuestions**：无（设计已收敛+对抗证伪+run1 验证 deliverable:true）。
  - **全文设计 rationale**（为什么不取方案B 等）：workflow 输出 `/private/tmp/claude-501/-Users-mac28-workspace-rustWorkspace-helix/25ed0a01-4885-4b70-be32-d8f054edd02d/tasks/wdv5vx813.output` 的 `result.design`。
  - 落完：commit(结构化 5 段) + `docs/ROADMAP.md` 把 #4 从 P 划掉 + 候选 HX-C 过回放闸门(倾向 HOLD·单样本)。

🧭 **续接一句话（新会话开局读这条·2026-06-22）**：在 worktree `worktrees/sync-v2-report`（分支 `feat/sync-v2-report-on-adr007`，off dev-zlc）—— sync-v2 **全 parity re-port + memberChange.leave 删行(StorageOp::BatchDelete) + 三未读/同步行为(冷启动 unread/在线增量/too_long-snapshot 覆盖式不删)** 全部落地，`cargo test` 全绿 + gate.sh/structure-gate 过，**真机冷启动路径已验收**（:8065 真连：increment 257 channel / 2275 member / member_count 实数 / 离线 backfill 950 msg / cursor 23）。
  **✅ A 已完成（host-cli stdin 遥控器 Tier0+Tier1，commit 333e977+621f086）**：8 命令 `send/read/revoke/leave/schedule/cancel/create/makeTopic`(+quit/help) → helix-im `im_*` → `Effect::Http` POST 现网真源 endpoint。chassis 运行时已证（piped help/quit→优雅退出）。
  **➡️ 下一步（三选一）**：
  - **A'（小·收口 A）真机 13-action e2e**：起 Go(:8065) + grpc(`xpa-2-12-grpc`) → `scripts/dev-loop.sh` 跑 host-cli → stdin 敲命令观察 WS 回推（posted-echo/post_read/post_update/channel_member_update/leave→BatchDelete/channel_created/schedule×2）。channel_close/nickname 无干净自 endpoint → 第二 actor(真 cses-client)触发观察。**本会话未跑（Go 未起）**。
  - **B（大·M2 ROADMAP P0）**：Tauri 前端四接缝接入 + 灰度开关 —— emit 消费(`im:__bus__`)/读路径 S7(2D cursor 分页)/too_long 前端 getLatestPost 链 的唯一验法。host-cli 是其同构参照（仅最外层 I/O 边界 stdin↔IPC 不同）。
  **真机连法**：`HELIX_DEVICE_ID` 已写死 `ED380414932542BAB4993E5AC3E07C64`（host-cli，env 可覆盖）· `scripts/dev-loop.sh` · 后端 grpc 服务名 = `xpa-2-12-grpc`（Go `grpcx9.go:66 CsesGrpc`，改名根因见 `docs/sync-v2-report/真机验收-grpc阻塞根因.md`）。**结构性未通**：Tauri 前端(M2)/FFI 手机端 IM e2e(M3-F4)/Snapshot(A2 服务端未实现不可达)。

🎯 EXPECT host-cli stdin 遥控器(Tier0 chassis)+helix-im outbound 命令全集(Tier1)解锁 13-action 自驱动真机验收 @2026-06-22 | 赌注 中 | 可逆 是（branch feat/sync-v2-report-on-adr007 未 merge）
  ↳ 可证伪预判：① 改动文件集 = host-cli(main.rs + 新 repl.rs stdin 解析) + helix-im(module.rs dispatch 臂扩 · acl/from_tick.rs accepts 扩 · 新 outbound handler edit/leave/create/schedule/cancel · acl/to_effect* 新 HTTP builder) + tests；**不碰根 SESSION/CLAUDE/README · 不碰 core ports/effect**（预测复用 Effect::Http/Persist/Emit/ScheduleTimer，不新增 Effect 变体）
  ↳ ② core 仍零运行时（`cargo tree -p helix-core` grep tokio/sqlx/reqwest/web-sys/tauri 空）· helix-im 不 import helix-driver-*（grep 验）· stdin 用 tokio::io 仅在 host-cli（host 层 ambient 合法）
  ↳ ③ stdin chassis 与 Tick::Inbound **同一条 tick_tx** 串行入泵（零并发进 core，守 HX-C002）；坏命令行 → 打印用法**不崩泵**
  ↳ ④ 每个 outbound 命令 = 解析 payload → `Effect::Http` POST 到 Go，**endpoint/method/body 取自现网 full-map backend HTTP 真源（§4.5），绝不编造**；找不到真源的 action → 文档化延后不猜；热路径无 RMW（HX-C005）
  ↳ ⑤ 新增可证伪测试：每个 outbound 命令 → 断言产出对应 `Effect::Http(正确 url)`（删 push → 测 FAIL，HX-C011）· read/reconnect dispatch 接通（命令名 → 非 "unhandled" 分支）· 坏命令不崩
  ↳ ⑥ cargo check 5 crate + cargo test core/im/host/native 全绿 · gate.sh + structure-gate 过（新 repl.rs / handler ≤300 行）
  ↳ 偏离即记 DRIFT-PRED：若需改 core Effect/ports / helix-im 反向依赖 driver / endpoint 编造非现网 / 测试非证伪 / src 文件 >300 行未拆 / 改根黑名单 → 偏离

✅ DONE outbound 请求体对齐真机 curl 真源 + 真机 e2e 全链验证（workflow fix-outbound-wire-then-e2e）@2026-06-22 | commit 45a621c(真源doc) + d1a9d0c(wire fix) | helix/feat/sync-v2-report-on-adr007
  ↳ 真源：用户提供现网实抓 6 curl → `crates/helix-im/docs/csesapi-migration/真机curl真源.md`（渐进索引 4 处）。揪出 send 真机无回声真因 = **请求体不对（snake_case + 缺 11 字段）·非 Pulsar**（用户证实 Pulsar 没问题）。
  ↳ 修（workflow Phase1·rust-tauri-skeleton-dev）：handle_send_message body 全 camelCase + 必填齐（type/simpleMessage/viewers/userId/teamId/userSnapshot…）· ImConfig +company_id（host 注入）· +im_channel_member_change(channel/member/change) · main.rs +appVersion/osVersion 头 + 13 头块收口 identity_headers(321→278) · post echo 对账链核实已存在(reconcile_post_echo)。cargo 5-crate + 30 test + 零运行时 + gate 全绿。
  ↳ 验（workflow Phase2·真连 Go :8065 自建频道 xf8mdqjc 全链）：**8 PASS / 0 FAIL / 2 SKIP·reached100**——✅ create/send(post 回声·核心)/revoke/makeTopic/nickname/schedule/cancel/close；⏭ read→post_read(自建频道无未读·数据依赖·Go 200)·member→channel_member_update(广播给目标用户非 self·单 client 观察限·Go 200)。2 SKIP 均非 bug（wire 正确）。
  ↳ 待联调核：deviceId 形态(连字符 UUID vs 32hex) · userSnapshot 完整 org/dept/userName 注入 · read/member 两 SKIP 需未读数据/第二 client 才能观察。

✅ DONE channel_close/nickname 补全 + host-cli↔Go e2e harness/workflow（事后补记·无开局 EXPECT：反应式纠错 + 用户中途请求展开）@2026-06-22 | commit 8b21ef9 + a6de5bf | helix/feat/sync-v2-report-on-adr007
  ↳ 纠错：上轮误判「channel_close/nickname 无干净自 endpoint」——实查 full-map #27/#31 均有自服务 endpoint 且 broadcast 到 channelId（自己也收）→ **13 action 全 host-cli 自驱动，无一需第二 actor**。补 im_channel_close + im_update_member_nickname（commands.rs test 外迁 tests/outbound_commands.rs 守 ≤300）。
  ↳ e2e 资产：`scripts/host-cli-go-e2e.sh`（逐 action 经 stdin 发命令 → grep HELIX_DUMP_FRAMES 入站帧 WS action 断言 → PASS/FAIL 表 + postId 链式 + 破坏性 leave/close 仅 CLOSE_CHANNEL_ID 给时跑）+ 具名 `Workflow({name:"host-cli-go-e2e"})`（Drive+Diagnose）+ main.rs emit 打印 event 名（im:*）便于断言。验的是**共享 driver-host I/O 栈**，跑一次（native/ffi 复用，端增量靠 smoke.cpp+F5）。
  ↳ 验证：cargo test im+host-cli 全绿 · gate.sh + structure-gate PASS · bash -n + 缺前置 exit 2 + node --check 均过 · **未 live 跑（Go 未起）**。
  ↳ ⚠️ 待办：起 Go(:8065)+grpc 后跑 e2e；host-cli main.rs:67 硬编码真 deviceId（drift-review 🟡 finding，留用户决定是否移 git-ignored 配置——见下 review 区间说明）。

✅ DONE host-cli stdin 遥控器 Tier0+Tier1（代码侧落地 + chassis 运行时实证）@2026-06-22 | commit 333e977(T0)+621f086(T1) | helix/feat/sync-v2-report-on-adr007（未 merge）
  ↳ Tier0：repl.rs(stdin→Tick::Command 复用 tick_tx 入泵·quit/EOF→shutdown) + host_env.rs(从 main 抽 db/deviceId) + main.rs 396→285 脱白名单。send/quit/help。
  ↳ Tier1：helix-im **新 commands.rs**(handle_outbound + is_outbound · 7 命令→Effect::Http POST 现网真源 endpoint) + module.rs dispatch is_outbound 分支 + from_tick accepts 扩；host-cli **新 cmd_parse.rs**(7 verb 解析，repl 320→拆 82+241)。命令：read/revoke/leave/schedule/cancel/create/makeTopic。
  ↳ endpoint 锚现网（full-map partials/{1,2}）：post/read · posts/revoke · channel/member/leave · posts/createSchedule · posts/cancelSchedule · channel/create · posts/makeTopic。零编造。
  ↳ 验证：cargo check 5 crate + cargo test im+host-cli 全绿（commands.rs 6 测锚 url+body·缺必填→Err·删 push→FAIL HX-C011；cmd_parse 6 测 verb→payload·动态键手建 Map·坏命令不崩）· gate.sh + structure-gate 全过 · core 零运行时空 · im 无 driver 依赖。**chassis 运行时实证**：piped `help/bogus/quit` → 用法打印 + 坏命令不崩 + quit→泵优雅 drain 退出 exit 0。
  ↳ EXPECT 对账：②③④⑤⑥ 全 held（零运行时/同泵串行/真源 endpoint/可证伪测试/gate+≤300）。①**部分偏离**（见下 ⚡）。**未碰** core ports/effect（复用既有 Effect::Http，0 新变体）+ 未碰根黑名单 ✅。
  ↳ ⚠️ **真机 13-action e2e 未跑**（本会话 Go/grpc 未起）——代码侧 + chassis 已证；真机待 Go 起后跑：`scripts/dev-loop.sh` 连 :8065，stdin 敲命令观察 WS 回推。channel_close/nickname 无干净自 endpoint → 走第二 actor 观察（已文档化延后，非编造）。
⚡ DRIFT-PRED EXPECT① 文件集预判部分偏：(a) 预测 outbound handler 落 module.rs/acl/to_effect，实际**新建 commands.rs** 收口（更干净·避免再涨 1159 行 module.rs / 748 行 to_effect.rs 白名单文件）；(b) 预测「新 repl.rs」单文件，实际为守 300 硬顶**额外拆 cmd_parse.rs + host_env.rs**（repl 一度 320 超限）；(c) 「edit」语义落地为 **revoke**——现网无「纯文本编辑」endpoint 产 post_update，post_update 由 revoke/quickReply/updateProps 触发，选 revoke(撤回) 作最干净自驱动 → 候选教训：「outbound 命令 EXPECT 文件集应预留『新建 commands 模块 + ≤300 拆分溢出文件』；action 名（edit）须先核现网有无对应 endpoint 再写进 EXPECT」。已 type:surprise 落 logs，回放闸门待过（结构性偏离·非架构 drift，疑不达建卡阈值）。

🎯 EXPECT #1/#2/#3 未读/同步三行为完整对齐现网 + 锁可证伪测试 @2026-06-22 | 赌注 中 | 可逆 是（branch feat/sync-v2-report-on-adr007 未 merge）
  ↳ 现网真源裁定（rust-tauri-skeleton-dev）：①冷启动 unread = 服务端权威 `unreadCount`（Some→写/omit→回退本地，channel_state.rs:292）②在线增量 = +1 per 新消息 + sender 豁免 + `create_at>last_root_post_at` 时间闸防历史重算（message_service.rs:365）③**too_long/snapshot 不物理删 message**——覆盖式 upsert + cursor 重置 + emit，前端 getLatestPost 重拉（post.rs:931 + im-sync.service.ts:61）
  ↳ 可证伪预判：① emit_sync_too_long 加 reset_to → payload `{channelId, resetTo}`（对齐现网；2 调用点 module.rs 服务端值 / channel.rs 客户端溢出 cursor+1 同步）② **绝不**加 `DELETE FROM message`（加了即违现网）；Snapshot 分支文档化「A2-reserved + 治理-as-Events + no-DELETE-intentional」③ 三可证伪测试：too_long emit 含 resetTo · guarded_bump 守卫 FAIL(historical create_at≤水位)→不 bump(host conformance) · sender 豁免(auth set 自己消息→unread_delta 0) ④ #2 冷启动 unread server-authoritative + omit-fallback 已测/补 ⑤ cargo check 5 + test core/im/host/native 全绿 + gate + structure
  ↳ 偏离即记 DRIFT-PRED：若加 message DELETE / emit 仍缺 resetTo / 测试非证伪 / 改根黑名单 → 偏离

✅ DONE #1/#2/#3 未读/同步三行为完整对齐现网 + 锁可证伪测试 @2026-06-22 | commit 5b089f1 | helix/feat/sync-v2-report-on-adr007
  ↳ 真源裁定后落地：#1 在线增量（+1 per 新消息 / sender 豁免 / `create_at>last_root_post_at` 防历史重算）**已对+已测**（post.rs 内联 5 测自己/他人/不可见/NOTICE + host guard pass/fail）· #2 冷启动 unread = 服务端权威 `unreadCount`（Some→写/None→omit 回退本地）**已对+已测**（program_full S3）· #3 too_long/snapshot **覆盖式非删**（现网铁律）
  ↳ 真实修复：`emit_sync_too_long` 加 `reset_to` → payload `{channelId, resetTo}`（原缺 resetTo + `channel_id`snake→`channelId`，对齐现网 post.rs:918 前端 cursor=resetTo-1 记账契约）；2 调用点（module.rs 服务端值 / channel.rs 客户端 buffer 溢出 cursor+1）；Snapshot 分支文档化（A2-reserved + no-DELETE-intentional + 治理-as-Events）
  ↳ ⚡关键纠偏（现网 agent）：too_long/snapshot **不物理 DELETE message**——覆盖式 upsert + cursor 重置 + emit + 前端 getLatestPost 重拉。我初判「snapshot 缺 DELETE 清表」**是错的**；物理 DELETE 仅 channel_member（成员全替换）/ event_cursor（自愈）。已写进 emit/Snapshot 注释防后人误加
  ↳ 新增 2 可证伪测试：s6 too_long emit 含 `resetTo=100`+`channelId` · host guarded_bump 守卫 fail（historical create_at≤水位）→ unread/水位 均不动。cargo check 5 + test 全绿（45 suites/0 fail）+ gate.sh + structure-gate ✅
  ↳ ⚡ DRIFT-PRED（drift-review wf_efcf1ca4-759 #11 补记·惊讶吞没纠正）：上方 :53「初判 too_long/snapshot 物理删 message=错 → 现网覆盖式 upsert+cursor 重置」是**真实预期偏离**（赌前以为要 DELETE），却被回填进开局 EXPECT②「绝不加 DELETE」+ 本 DONE 原自评「5 断言全 held·无 DRIFT-PRED·开局对账机制按设计生效」= §4.8 惊讶吞没后门。如实改判：**此为 DRIFT-PRED**，偏离裁定权归 drift-review 独立 lens、非生产者自评。候选教训：too_long/snapshot 语义先验现网 upsert-vs-delete 再写 EXPECT。已 type:surprise 落 logs。

🎯 EXPECT 实现 StorageOp::BatchDelete + 接通 memberChange.leave 删行 @2026-06-21 | 赌注 中 | 可逆 是（branch feat/sync-v2-report-on-adr007 未 merge）
  ↳ 可证伪预判：① 改动文件集 = core(effect.rs +BatchDeleteSpec/StorageOp::BatchDelete · ports/storage.rs +batch_delete trait) + driver(host ops.rs 派发+表名 arm · host/native/wasm storage.rs +batch_delete impl) + im(acl/to_effect_s1.rs +delete_channel_members · ws/handlers/channel_member_update.rs + increment_channel.rs 接 collect_member_leaves) + tests；不碰根黑名单
  ↳ ② SQL = DELETE FROM channel_member WHERE channel_id=? AND user_id IN(?…)（复合 PK 作用域，O(k) 单语句，绝不跨 channel 误删）
  ↳ ③ core 仍零运行时（`cargo tree -p helix-core` grep 空）· native 委托 host · wasm `todo!`（归档不交付）
  ↳ ④ 新增可证伪测试：channel_member_update 带 memberChange.leave → 断言 BatchDelete op on channel_member 含 leaver user_id（删调用 → 测试 FAIL，HX-C011）
  ↳ ⑤ cargo check 5 crate + cargo test core/im/host/native 全绿 · gate.sh + structure-gate 过
  ↳ 偏离即记 DRIFT-PRED：若 core 出运行时依赖 / delete 无 channel 作用域跨群误删 / 测试非证伪 / 改根黑名单 → 偏离

✅ DONE StorageOp::BatchDelete + memberChange.leave 删行 @2026-06-21 | commit 975a5d7 | helix/feat/sync-v2-report-on-adr007
  ↳ 落地：core(StorageOp::BatchDelete + BatchDeleteSpec + batch_delete trait) · driver(host 真 SQL `DELETE … WHERE channel_id=? AND user_id IN` / native 委托 host / wasm todo! 归档) · im(delete_channel_members 工厂 + channel_member_update/increment 接 collect_member_leaves，channel_created 经委托覆盖)
  ↳ 验证：cargo check 5 crate + cargo test core/im/host/native 全绿 · gate.sh + structure-gate ✅ · core 零运行时 0 · 2 可证伪测试（s1 leave→BatchDelete · storage_conformance 真 SQLite 跨作用域隔离 c2/u1 不误删）· s6 too_long「不删 message」强化
  ↳ EXPECT 5 断言全 held（文件集 / SQL 作用域 / 零运行时 / 可证伪 / gate 全中），无 DRIFT-PRED。填补原「memberChange.leave 解析无消费方」ROADMAP-P 缺口（core 原无 Delete）

🎯 EXPECT sync-v2 全 parity re-port 到 dev-zlc(ADR-007) @2026-06-18 | 赌注 高 | 可逆 是（branch feat/sync-v2-report-on-adr007 未 merge）
  ↳ 可证伪预判：① 改动文件集 = Phase A(increment_channel/increment_channel_end/module.rs) + B0(helix-core effect.rs/ports·storage.rs + helix-driver-{host,native,ffi,wasm}/storage.rs) + B(channel_write/5 + acl/to_effect{,_s1}.rs + parser/context/schema/sync_session/ImConfig) + C(ws/handlers/10 新 + mod.rs + 改 post/increment) + D(tests)；**不碰根 SESSION/CLAUDE/README**
  ↳ ② 每阶 feat 对应 replay 测试逐字 port 且 PASS（http6/b2/s1(576)/s4/s5/s6/channel_write_program/channel_member_collect 全绿）= parity 硬证
  ↳ ③ base64 decode 在 helix-im 内联（grep helix-im 不 import helix-driver-host）· core 仍零运行时（`cargo tree -p helix-core | grep tokio|sqlx|reqwest|web-sys|tauri` 空）
  ↳ ④ unread = GuardedBump SQL 自增（grep 无 path3 read-modify-write）· member_count = 单行 ON CONFLICT upsert（HX-C005）
  ↳ ⑤ cargo check 5 crate + cargo test helix-im/core/host/native 全绿 · gate.sh + structure-gate 过（新 src 文件 ≤300 行）
  ↳ ⑥ #5 auth 不搬（ImConfig 无身份字段）· S9 emit A5/A4 分流落地（grep `im:channel:read_echo` + `im:post:batch-updated`）· 全 parity 真机在线+离线（localhost:8065）跑通
  ↳ 偏离即记 DRIFT-PRED：若 core 出运行时依赖 / base64 反向依赖 host / path3 RMW / feat 测试未 port 或不过 / 改根黑名单 / src 文件 >300 行未拆 → 偏离

✅ DONE sync-v2 全 parity re-port 到 dev-zlc(ADR-007) · 代码侧完成 + 确定性验证 @2026-06-18 | commit 36f65a5..afad24b（9 phase + 1 parity-fix）| helix/feat/sync-v2-report-on-adr007（未 merge）
  ↳ 落地：A(#1 cursor 种子0 + #6 ADR-007 信封内联 base64 解层 + HX-C010 sort) · B0(GuardedBump substrate 跨 5 crate) · B(channel_write 5 文件 + acl to_effect{,_s1} + 成员 + member_count + 前置基建) · C(10 WS handler · S1 全 14-action · S9 emit A5/A4 分流 · linkme wiring) · D(makeTopic + S4/S5/S6 测试 + S7/S10 验 + #5 核实不搬)
  ↳ 验证：cargo test core/im/host/native **277 passed / 0 failed**（feat replay 测试 http6/b2/s1/s4/s5/s6/channel_write/channel_member 逐字 port 全绿）· gate.sh + structure-gate ✅ · core 零运行时 · base64 helix-im 内联无 host import · unread=GuardedBump O(1) · A5 `im:channel:read_echo`/A4 `im:post:batch-updated` 落地 · #5 auth 不搬（全局头）· channel_write 各文件 ≤300
  ↳ EXPECT 对账：①③④⑤ held；②**falsified**（见下 ⚡）；⑥代码 held 但**真机验收 env-blocked**——后端 Java paas grpc fail @cses_channel.go:177（非 re-port；deviceId/session 已通），待 grpc 恢复跑在线+离线真机验收
  ↳ review：per-phase 4 专家 pass（1 Phase C blocking A5 分流已修 e9e6c64）+ holistic drift-review（rust-dev call-site lens）抓出 2 parity 死代码 call-site → afad24b 修 + 补可证伪测试。🟡 轻 drift，无 HX-C/GOAL 违反，多数 src byte-identical 于 feat
⚡ DRIFT-PRED EXPECT②「每阶 feat 测试逐字 port 且 PASS = parity 硬证」**被证伪**——byte-diff + per-phase ported 测试证不了消费侧 call-site 是否接线：2 处「函数在(byte-identical)、零调用」死代码（sync_unread_bump_ops 在 module.rs SyncPull 零调用=BLOCKING 离线未读丢 / host auth_user_id 恒空=MAJOR 自己消息计未读）藏在 ADR-007 接缝文件，cargo 绿 + 277 测试绿仍漏，靠 holistic call-site/caller-graph 独立 lens 才抓出 → 候选教训：「parity re-port 验收必加 call-site/caller-graph 审计 lens（grep 函数定义 vs 调用方），不只 byte-diff + ported 测试」。已 type:surprise 落 logs，回放闸门待过再建卡

✅ DONE 真机在线+离线 parity 验收**通过** @2026-06-21 | helix/feat/sync-v2-report-on-adr007 真连 localhost:8065（EXPECT⑥ 真机部分现 held）
  ↳ 落库实证（fresh DB 冷启动 = 离线 fromSeq=0 backfill）：**channel 257 行** · **channel_member 2275 行**（MEMBER 2058/OWNER 214/BOSS 3，四源+memberChange.join role normalize）· **message 950 行**（离线 backfill）· **channel_event_cursor 23**（max seq 554）· **member_count 实数 599/598/41/40…**（原卡 0 现真值）· Go「Increment worker completed」· increment_channel handler ×257 dispatch
  ↳ 全 parity 闭环点亮：#1 cursor 种子0 + #6 解信封 → 离线 backfill 真落 950 msg · B0 GuardedBump + channel_write → 257 channel · #2 成员 → 2275 member · member_count → 实数 · S2 cursor → 23 行
  ↳ 阻塞根因（非 re-port · 详 [`docs/sync-v2-report/真机验收-grpc阻塞根因.md`](docs/sync-v2-report/真机验收-grpc阻塞根因.md)）：后端 grpc serviceName 改名 `xpa-12-grpc`→`xpa-2-12-grpc`（consul 0 实例 → 用户起服务+改 Go `grpcx9.go:66` const 后修复）。deviceId 写死 `ED380…`（host-cli env 可覆盖，d966250）

🧭 **续接一句话（新会话开局读这条）**：在 worktree `worktrees/sync-v2-report`（分支 `feat/sync-v2-report-on-adr007`，off dev-zlc）把 feat 的 **sync-v2 整条业务实现 re-port 到 dev-zlc 的 ADR-007 架构，目标 = feat 全 parity（不只 6 个 fix）**。参照源 = `feat/sync-v2-full-replacement`（已真机验证，**不文本合并**，逐项 re-port）。

  ⚠️ **范围铁律：目标是 feat 全 parity，A+B 只是子集，远不够。** 之前映射 agent 只读 dev-zlc、没 diff feat → 漏了一半。**第 0 步必做**：

  - **Step 0（前置·必做）全枚举审计**：`git diff dev-zlc...feat/sync-v2-full-replacement -- crates/` **读两边**，列出「feat 有 / dev-zlc 缺或不同」的**每一项**成真 re-port 清单（别只搬记得的 6 个 fix）。已知 dev-zlc 缺的大块：**8 个 WS action**(post_read/post_update/posts_update/update_channel/channel_created/channel_close/channel_member_update/nickname/schedule×2，S1)、**channel 三路径写整层**(S3)、**成员 memberChange.join**(#2)、**member_count**、**makeTopic**(#3)；以及 **S2 cursor 表/S4 写顺序/S5 echo/S6 sync 续拉/S7 读契约/S9 emit 全集/S10 命名陷阱** 逐项核「dev-zlc 已有还是要搬」。
  - **分阶 re-port（覆盖全 parity，每阶 cargo + 真机 e2e）**：
    - Phase A：#1 cursor 种子=0 + #6 解信封层（先修好 dev-zlc 现带的两个数据丢失 bug，且验 ADR-007 接缝适配）
    - Phase B：#2 成员 + #6col channel 三路径写 + member_count
    - **Phase C：缺失的 8 个 WS action（S1 全 14 对齐）**——post_read/编辑/update_channel/channel_created/close/成员/排程
    - **Phase D：#3 makeTopic + S4/S5/S6/S7/S9/S10 核对补全**（写顺序铁律/echo 三级/sync 续拉 too_long/读契约二维游标/emit 全集对齐/命名陷阱）
    - #5 auth：**不搬，仅核实** dev-zlc 全局头(cookieId)已覆盖鉴权
  - **全 parity 验收**：feat 全部用例在 dev-zlc(ADR-007) 上真机跑通——在线收发+echo回填、离线 backfill、编辑/已读/撤回/排程、makeTopic、channel/成员落库、14 action。
  - **每个 commit 检查点先跑 dev-zlc 分叉监控**（见下），落后就先 rebase。

📌 **背景/坑（2026-06-18，必读，别再踩）**：
  - **架构并行分叉**：我 10:24 从 8997a72 分叉建 feat/sync-v2 做业务层；**另一个会话 12:43 把 ADR-007（engine 下沉 host、native/FFI 薄壳、http_envelope 信封契约）合进 dev-zlc**。两条线整天并行没碰头 → 17:33 才发现架构岔开巨大、而**业务代码两边都已开发**。教训：**开长线前先对齐基线**（都从 dev-zlc 最新分叉 / 定期 rebase），否则岔几小时就分道。
  - **好消息**：helix-im 是 sans-IO，业务逻辑（#1/#2/#6col/#3）与 driver 架构无关，re-port 干净；只 #6（解信封）碰 driver 接缝。
  - **哪个架构留**：ADR-007（dev-zlc）更好（服务北极星「一份 core 编 native+FFI」），保留它；搬 feat 的业务逻辑过去。
  - **信封契约已拍板**：保 dev-zlc 的 ADR-007 信封，#6 改成「parse_sync_response 前加解信封+base64 层」（落 module.rs handle_sync_reply，只对 SyncPull 路径，别碰 scan）。base64 跨 crate：helix-im 不能依赖 host → im 自带 decode 或挪 core。
  - **#5 不搬**：dev-zlc 用全局头(cookieId)已覆盖，搬进 ImConfig 违 ADR-010。
  - **re-port 落点矩阵** 见映射 agent 报告（dev-zlc #1 现带 lastEventSeq 种子 bug=increment_channel.rs:22 / #6 latent 吞数据 bug=module.rs:491 / #2+#6col channel-member 写层整体缺失 / #3 makeTopic 待读 feat diff）。

🔧 **真机联调铁律（今天反复踩，沉淀）**：
  - **所有 HTTP 请求头都要带 deviceId**（`HELIX_DEVICE_ID`）——缺了对端 paas 判 session 过期 → grpc 拒。今天踩好多次。host-cli 已支持 env 注入。
  - **真机连 Go 用 `scripts/dev-loop.sh`**（焊死正确 base + 双端日志三层关联），别手搓 host-cli env：base 必含 `/api/cses`、`HELIX_DB` 别带 `?mode=rwc`（host-cli 自拼）。
  - **后端三仓 + grpc 排障**：Go mattermost = `/Users/mac28/workspace/golangProject/mattermost`（日志 `/Users/mac28/workspace/mmlog/mattermost.log`）；Java cses(paas/x9 grpc) = `/Users/mac28/workspace/java/cses`。出 grpc 问题（`no service available` / `RunIncrementWorker panic` / `new grpc client fail` / consul `xpa-12-grpc` 0 实例）→ **查各自日志**；常见根因 = Java paas grpc 服务挂/未注册 consul（今天 17:19 因一个 JVM suspend=y 等 debugger 卡住）。

🔁 **dev-zlc 分叉监控 → 已硬化成机器闸门 `scripts/branch-sync-gate.sh`**（drift-review #2 收敛 / 用户 2026-06-22 指出散文太软）：旧版只是本条散文「落后 ≥5 就 rebase」、无脚本无 hook → 静默失效（feat 一度落后 dev-zlc 17）。现跑 `bash scripts/branch-sync-gate.sh`——当前分支落后集成分支（默认 dev-zlc，env 可覆盖）≥5 commit = exit 1 强制先 `git rebase dev-zlc` / `git merge dev-zlc` 再续。建议挂 pre-push / Stop hook / 开局检查（与 review-gap.sh / structure-gate.sh 同族）。

✅ DONE sync-v2 全链真机闭环（在线+离线）@2026-06-18 | feat/sync-v2-full-replacement（参照源，未合并）：删库重拉离线 backfill 实证（Go `kind=events fromSeq=0`→message 落行）+ 在线发收 echo 回填。6 fix + member_count 全 commit、cargo/test 绿。**唯落到 dev-zlc 的 re-port 待做（见上续接）。**
✅ DONE 生命周期与网络 I/O 解耦根治 + ffi-verify 6 层闸门落地 @2026-06-22 | commit 448a993..4bfff14（worktree fix/lifecycle TDD → merge c10b2a9 → ffi-verify 4bfff14）| helix/dev-zlc
  ↳ 🎯 EXPECT 真源 = **worktree REPORT.md**（开局写·git `448a993` 可查·**并行规划支柱③：worktree 线 EXPECT 落 REPORT、集成者汇总 DONE 到根 SESSION，非孤儿 DONE**）
  ↳ 根治（systematic-debugging Phase1-4 确证 + worktree TDD 红→绿）：连不上服务器时 helix_create/helix_destroy 各卡 30s（本机 5s=OS TCP connect timeout 先于 reqwest 30s 应用层触发）——① create `ws.connect` 异步化（pump spawn 后台连接·ready_tx 立即回传，兑现「连不上仍进泵」）② destroy 必达 Http drain 上限（spawner `shutdown_with_timeout` + engine `HTTP_DRAIN_LIMIT=3s` 超时 abort）。**五不变量⑤ 改写**「drain=必达 Http 已持久化(乐观态)·落地靠重发，非必等满网络」+ host/native CLAUDE/AGENTS 4 文件镜像
  ↳ HX-C011 双向证伪铁证：①还原同步 connect→FAIL@5s / 根治 PASS@0.02s；②还原无上限 drain→FAIL@4.99s / 根治 PASS；两测各守一根因·互不串扰
  ↳ ffi-verify.sh 落地（6 层端线自验·**并行规划支柱④**）：**首跑即抓到此 destroy bug（脚本价值自证）**→ 根治后全 6 层 PASS 24.68s（e2e 60s→0.23s·cpp-smoke timeout→0.06s·console PASS）；gate.sh 第3步补 ffi test + --full 加 cpp-smoke
  ↳ 验证：FFI 17 + native stress 6/6 + host 17 全过·core 零改(grep 空)·clippy 零新 warning·structure-gate·黑名单(core/im/根)零触碰·与活跃 sync-v2-report 零交集 merge·gap=4 绿
  ↳ EXPECT 6 断言对账：5 命中 + #4「真 Go 连得上时必达 Http 落地」⚠️**部分**（本环境无真 Go·结构化论证替代：drain 上限 3s≫LAN POST 往返·连得上不 abort 落地路径不变）→ **backlog：go+grpc 真连补验 #4**
  ↳ ⚡ DRIFT-PRED ①（根因数值+复现源）：开局赌 30s 实测 5s（TCP connect 层 OS timeout 先触发）；`*.example.test` 被本机 DNS 重定向 fast-refuse 假绿(0.02s)测不出阻塞→改 blackhole IP `192.0.2.1`(RFC5737)才真复现。候选教训：**timeout bug 复现用稳定 hang 的 blackhole IP，非 DNS-refuse 域名**
  ↳ ⚡ DRIFT-PRED ②（测试覆盖）：多根因 bug 须**逐根因独立证伪**（单聚合断言碰巧覆盖①漏②，还原②无上限 drain 仍假绿）→补 `destroy_not_blocked` 独立测。**HX-C011 延伸**，候选并入（下次 drift-review 评估）

✅ DONE 修复 backlog：建 HX-C012（候选）+ f5 F1/F2 文档诚实化 @2026-06-21 | commit 3418ac7..a409eea（A 建卡 / B f5 / C 自指 recurrence） | helix/dev-zlc
  ↳ 候选 HX-C012：**用户拍板**建卡——实质任务（含 test/CI/harness ≥50 行或新源文件）须**开局**写 🎯 EXPECT·禁事后补记（堵 DRIFT-PRED 永空 audit 后门）；drift-review wf_fd406543-c76 回放闸门 **HOLD** 在卡 §7 诚实记录（历史 trap 均代码级·与流程治理缺口性质不同源）；索引 11→12 四处同步（卡数==README==CLAUDE==AGENTS==12 + 根镜像 diff 空）
  ↳ f5：egress_parity.rs 头注/doc 诚实化——F1 收紧措辞：证的是 **egress 管道 parity**（batch roundtrip HX-C007 vs broadcast 对同一 source byte-identical）非端到端引擎双端等价（引擎靠共享 build_shell by-construction）；F2 doc `subscribe_observed()`→裸 `try_recv` 如实（与 host-cli main.rs:170/243 对齐）+ FfiEventSink 无 lagged/resync 登记前移 M2；**零逻辑改·f5 两测 2+2 passed**
  ↳ ⚡ 自指（无配对 EXPECT 的诚实承认）：本「修复候选+f5」实质任务（6 文件跨 docs/test）**自身未开局写 EXPECT** = 刚建的 HX-C012 第 3 次同根复发 → recurrence 2→3·log.md+§5 如实记（**非补假 EXPECT，守 §3 Forbidden**）·同根达 3 部分缓解回放闸门 HOLD。本 DONE 即「孤儿 DONE」实例，留作 HX-C012 反面教材
  ↳ 验证：harness 索引==12 四处一致 + 根 CLAUDE/AGENTS 镜像 diff 空 · structure-gate PASS · f5 两测全过 · core/im 零逻辑改

🎯 EXPECT F4.5 Flutter FFI 桌面 demo（C 路线：完整 Xcode 待装 → console 证链路 + UI/test 代码就绪）@2026-06-18 | 赌注 中 | 可逆 是（Flutter 工程在仓外 /workspace/flutterWorkspace/helix_ffi_demo，删目录即回滚；仓内仅 3 行文档）
  ↳ 可证伪预判：① 仓内只动 docs/superpowers/specs/2026-06-18-...（已）+ docs/flutter-demo-pointer.md + ffi CLAUDE.md/AGENTS.md 各 1 行索引，**helix-core/helix-im/cdylib 零改** ② `dart run` console 冒烟 PASS：dart:ffi→C++ shim→cdylib→pump→Emit→batch→`NativeCallable.listener` 异步回调收到含 `im:post:sending` 的批（证异步跨线程全链）③ build_native.sh 一键 `cargo build --release`(panic=abort) + `clang` 链 cdylib 产 libhelix_shim.dylib（otool -L 验 @rpath，**不碰 Xcode**）④ `cargo check -p helix-driver-ffi` 仍过 + `scripts/cpp-smoke.sh` 仍绿（零回归）⑤ structure-gate 镜像闸绿（ffi CLAUDE/AGENTS 成对 diff 空）⑥ integration_test/main.dart/maestro 代码就绪，`flutter test -d macos` 标 BLOCKED-on-Xcode（阶段 2，不计失败）
  ↳ 偏离即记 DRIFT-PRED：若被迫改 cdylib 导出 / shim C++ 异常外逸跨 extern C / listener 拿到失效 ptr UAF / console 证不了异步跨线程 / 仓内动了 core·im / free_batch 非无状态

✅ DONE F4.5 Flutter FFI 桌面 demo（C 路线·阶段1：console 证完整链路）@2026-06-18 | commit 195c88e..4db4592（spec+session+ffi 文档）+ 仓外工程 /workspace/flutterWorkspace/helix_ffi_demo | helix/dev-zlc
  ↳ 链路真证（阶段1）：`dart run bin/console_smoke.dart` PASS×3 — dart:ffi→C++ shim(libhelix_shim.dylib)→cdylib→pump→Effect::Emit→batch→`NativeCallable.listener` 异步回调收 1 条 101B 完整 JSON `{"event":"im:post:sending"}`（listenerFired=true / free_batch 调用 / dispose 6 步 join pump，无 UAF·无 hang·exit 0）；build_native.sh 一键 cargo --release + clang 链 cdylib，两 dylib @loader_path 同目录（免 DYLD_LIBRARY_PATH）
  ↳ Workflow 编排 9 agent：P1 脚手架×4 + P2 shim+Dart×2（**一次跑通零返工**）+ P3a console 验证 + P4 cpp-rust-systems-dev 闸门（🟢 7 不变量逐条带行号成立：malloc+memcpy 稳定化/noexcept+catch/整批一次/free_batch 无状态/uintptr_t↔UintPtr/销毁 6 步/C malloc-free 不混 new）+ 收口 gate（🟢）
  ↳ 交付物：仓外 helix_ffi_demo（native/helix_shim.{h,cpp} + lib/{helix_ffi,main}.dart + bin/console_smoke.dart + integration_test + maestro 骨架 + tool/build_native.sh）；仓内 3 文档（docs/flutter-demo-pointer.md + ffi CLAUDE/AGENTS 镜像各 1 行）
  ↳ 验证：cargo check -p helix-driver-ffi PASS(3.33s) + scripts/cpp-smoke.sh 零回归 PASS + structure-gate 镜像闸 PASS + core/im/.rs/Cargo.toml **零改**
  ↳ EXPECT 对账：6 条断言**全命中**（仓内仅 3 文档 / console PASS 见 im:post:sending / build_native @loader_path / 零回归 / 镜像闸绿 / UI·test·maestro 代码就绪）→ **无 DRIFT-PRED**（预期全中）
  ↳ 阶段2 BLOCKED-on-Xcode：`flutter test integration_test -d macos` 待用户装完整 Xcode+CocoaPods（本机仅 Command Line Tools，flutter doctor 实测）；UI/integration_test/maestro 代码已就绪，装后一键可跑
  ↳ 收口：gap=10 → §4.7 强制 architecture-drift-review（紧接派，通过后更新 ledger e3540cf→HEAD）

🎯 EXPECT F5 跨端 parity 自动化（同 Tick 剧本 native vs FFI byte-level 一致 · 设备无关）@2026-06-18 | 赌注 中 | 可逆 是（worktree feat/f5-cross-parity 未 merge）
  ↳ 可证伪预判：① golden Tick 剧本→Emit DomainEvent 序列 byte-identical 到 committed fixture + 跑两次确定性 ② native(NativeEventSink broadcast)↔ffi(FfiEventSink batch decode) 对同剧本 deliver 的 DomainEvent byte-identical ③ 改动只在 tests + golden fixture（不改 src 逻辑/不碰 core 行为）④ 两测 HX-C011 可证伪（改 emit / break egress → fixture mismatch FAIL，commit body 附破坏证明）⑤ cargo test 全绿 + native 零回归 + gate.sh 过
  ↳ 偏离即记 DRIFT-PRED：若改 src 逻辑 · golden 是 tautology(改 emit 照过) · parity 黑盒证不了 · 跨 crate dev-dep 成环 → 偏离

✅ DONE F5 跨端 parity 自动化 @2026-06-18 | commit ce8ccfc..7601e15（merge 7601e15）| helix/dev-zlc
  ↳ 两测：① helix-im/tests/f5_golden_replay.rs — Tick 剧本(im_send_message Command + posted Inbound)→shell.step→Emit DomainEvent 序列 == committed golden(fixtures/f5 ndjson 2 行)+ 跑两次确定性 ② helix-driver-ffi/tests/f5_egress_parity.rs(dev-dep native)— 同剧本 emit 经 NativeEventSink(broadcast) vs FfiEventSink(batch decode) deliver 的 DomainEvent **byte-identical**
  ↳ HX-C011 破坏→fail 实测：改 golden event 名→A FAIL/还原 PASS · 截断 FfiEventSink flush 末条→B FAIL(ffi 1 条 vs native 2 条)/还原 PASS（非 tautology）
  ↳ 验证：im+ffi 新两测过 · native 零回归 · gate.sh 全闸门 · core 零运行时 · 未改 src/未碰 core · ffi dev-dep native 无环(生产构建树不含 native·cargo tree no-dev 验)
  ↳ EXPECT 5 断言全 held；verify-diff 0 blocking。1 major(worktree agent 误改根 SESSION·§6.5)**已撤回收敛**、DONE 由主对话写。2 minor backlog：parity 专证 egress 机制(引擎双端一致靠共享 build_shell by-construction)+ 多 step 多 batch 拼接未覆盖(单批已足证)
  ↳ ⚡ DRIFT-PRED：本轮按 F3 教训开局写了 EXPECT ✅（流程改进生效）；新坑=worktree agent 仍越界写根 SESSION（§6.5 黑名单 prompt 已含却没拦住）→ 候选：派 worktree agent 时强调「§4.8 EXPECT/DONE 落 worktree REPORT.md 不落根 SESSION，根 SESSION 主对话独占」。已 type:surprise 落 logs

✅ DONE C++ 直连冒烟自动化 harness（cpp→rust→cpp 真 C++ 端到端）@2026-06-18 | commit a80cc49 | helix/dev-zlc
  ↳ 真 C++ link libhelix_driver_ffi.dylib + #include helix.hpp：create(vtable)→helix_command(im_send_message)→收 on_domain_events_batch 含 im:post:sending→destroy(join pump)。cmake/ctest 1/1 PASS 1.54s；scripts/cpp-smoke.sh 一键（CI 可调）；crates/helix-driver-ffi/smoke/{smoke.cpp,CMakeLists.txt}
  ↳ 环境现实：本机无设备/iOS 模拟器/完整 Xcode/Android SDK → **Flutter 真机冒烟不可行**；完整 Flutter↔C++↔Rust 链需用户原项目 C++ host。C++ 直连冒烟=本架构(C++-host)的「真机冒烟」正解。Flutter 安装后台进行(brew cask)，装后仅 macOS 桌面 demo 可跑

✅ DONE 收口代办 + 沉淀三端设计思想文档 @2026-06-18 | commit 7f28a19..daf44fe（merge 527100f + hotfix daf44fe）| helix/dev-zlc
  ↳ 代办全清：DropOldest→DropNewest 改名(13 文件) · native↔host 压测去重(共享核 4 测下沉 host stress_storage_core + native 2 parity 烟测·覆盖 6>4) · helix_query 设计落档(blocked-on-IM 通用 query 路由) · feat/sync-v2 path3 零 RMW 结论存档(两分支均原子 ON CONFLICT·unread 不写 channel 表)
  ↳ 沉淀：helix-driver-{host,ffi,native}/docs/design/00-设计思想.md（host 完整 8 节：北极星/parity by construction/C++直连pivot/BatchSink/读写并发/方法论教训/不变量/待办 · ffi+native 角色版引用 host）+ ffi/03-helix-query-design.md
  ↳ ⚡ DRIFT-PRED：merge 后 gate.sh 红——P1-A 墙钟下界断言 flaky（finalize worktree 侥幸过/dev-zlc 更快即 FAIL）。hotfix daf44fe 删下界(保留全完成/无死锁/非全串行非 flaky 断言)。教训「timing 断言天生非确定，不变量结构化证不靠墙钟」=「压测证伪证明」族**第 3 次同根**（2026-06-15 stress REJECT + storage false-green + 本次 flaky）→ **达 HX-C 建卡阈值(≥3)，下次 architecture-drift-review 过回放闸门建卡**。已 type:surprise 落 logs
  ↳ 验证：cargo test host+native 全绿 · clippy -D warnings 0 · core 零运行时 · structure-gate + gate.sh 全闸门
  ↳ 🎯 EXPECT（事后补记·十轮 drift-review F3 补偿·本应开局写）：可证伪预判——① 改动只在 driver-{host,native,ffi}/**（+ core 仅 DropOldest 文档残留、零代码逻辑改）② 改名 DropNewest behavior-neutral（cargo test 不破）③ 压测去重覆盖不减 ④ 三端设计思想 md 沉淀 ⑤ gate.sh 过。实际偏离：P1-A flaky 已记上方 DRIFT-PRED。**教训**：实质收尾任务亦须开局 EXPECT——§4.8 drift-review F3 抓出本 DONE 缺配对 EXPECT，DRIFT-PRED 一度悬空

🎯 EXPECT storage 读写并发压测全 phase（下沉后共享核覆盖 + sync-v2 写负载）@2026-06-18 | 赌注 中 | 可逆 是（worktree feat/storage-rw-concurrency 未 merge）
  ↳ 可证伪预判：① 改动只在 helix-driver-{host,native}/{tests,benches}（+ 仅 P0 命中 dev-zlc RMW 时 helix-im/channel.rs 一处），不碰 core/仍零运行时 ② P0 path3 未读自增确认为原子 `ON CONFLICT SET unread=unread+excluded`（预测合规非 RMW；若 RMW 则修）③ 6 缺口场景落地（读读 N>4 背压/WAL 脏读断言/多 channel cursor 单调/engine 混合 flood/FFI↔native bench）+ native↔host 迁移清单 ④ cargo test host 全绿 + native 零回归 ⑤ verify 无 blocking 才算成（新测须真证伪、无 false-green）
  ↳ 偏离即记 DRIFT-PRED：若动 core/im 超 P0 一处 · 新测 false-green(测了证不了) · 迁移丢覆盖 · path3 实为 RMW 却没修/没 flag → 偏离

✅ DONE storage 读写并发压测全 phase @2026-06-18 | commit b494e11..caa203a（merge cb8b6d5）| helix/dev-zlc
  ↳ 范围：host/tests 加读读 N>4 Semaphore 背压 + WAL 脏读可证伪断言 + 多 channel cursor 单调(含回退证伪) · native stress_engine 三入口混合 flood · P3 伪对照 bench 诚实删（存储层两 runtime 经 spawn_blocking 构造上等同，差异在泵层已 P2 覆盖）
  ↳ P0 结论：dev-zlc channel.unread_count 本轮无写路径=零 RMW（HX-C005 合规）；cursor advance=monotonic_upsert 原子 ON CONFLICT WHERE excluded>value（O(1)）；feat/sync-v2 path3 flag「合并 review 待查」未跨改
  ↳ 验证：host 34 + native stress 8 零回归 · clippy -D warnings 0 · gate.sh 全闸门 · core 零运行时 · **每条不变量带证伪证明**（P1-C 去 MAX guard→FAIL/还原→PASS · P1-A 去背压→FAIL）
  ↳ ⚡ DRIFT-PRED：EXPECT⑤「新测须真证伪/无 false-green」首版偏离——2 false-green（P1-C 顺序写恒 100=tautology·删 guard 照过 · P3 bench 无 assert + block_on 单 op 不跑并发）被 verify-diff test-validity lens 抓出 → 修+证伪证明。EXPECT③ 小偏离：P3 改诚实删非实现、迁移纯增量未去重(backlog)。候选教训「压测/并发不变量测试必带证伪证明(破坏即 fail)否则假信心」——与 2026-06-15 stress-verify REJECT 同根=2 次，未达建卡三铁律，记 logs 待回放闸门。已 type:surprise 落 logs

🎯 EXPECT driver-ffi M3-F4 engine pump 落地（命令真跑 + 事件回 C++ · 镜像 native）@2026-06-18 | 赌注 高 | 可逆 是（同 branch design/ffi-cpp-direct 未 merge）
  ↳ 【2026-06-18 scope 修订·用户拍板】grounding 揭示「下沉整套 pump 核到 host、native+ffi 都成薄壳」优于「confined-to-ffi 复制」（否则 HTTP 信封 + 刚统一背压纪律随时间漂移、破跨驱动 parity）→ 用户选下沉。故断言①作废、scope 扩为三 crate（host 得 pump 核 / native 重构薄壳 / ffi 新薄壳）。安全网 = native 测试零回归。
  ↳ 可证伪预判：① 改动在 helix-driver-{host,native,ffi}/**（helix-core/helix-im 仍不碰，core 仍零运行时）② pump 跑在 dedicated std::thread + tokio current_thread runtime（core 单线程跑 · GOAL §1.3）· native 重构后 cargo test -p helix-driver-native 零回归（下沉零行为改变安全网）③ Effect 分发镜像 native engine_loop 语义——尤其 Http 任意 status→PortOutcome::Ok(ReplyBytes 同 envelope)、只传输错→Err（跨驱动 parity）④ Emit 整批→on_domain_events_batch 一次过边界（HX-C007 · 不逐条）⑤ helix_query 走 spawn_blocking 非 block_on ⑥ helix_destroy drop-sender→join 干净退出 ⑦ cargo check/clippy/test 过、helix-core 仍零运行时
  ↳ 偏离即记 DRIFT-PRED：若 core/im 被改 · pump 用 multi-thread 把 core 推上多线程 · Http status 判 Err 破 parity · Emit 逐条跨边界 · query 用 block_on 死锁 → 偏离

✅ DONE driver-ffi M3-F4 pump 端到端接通（cpp→rust→cpp：命令真跑 + 事件批回 C++）@2026-06-18 | commit cfe7a2a..630a713（10 commit · branch design/ffi-cpp-direct 未 merge）| helix/design/ffi-cpp-direct
  ↳ 架构：W1 下沉 pump 核到 helix-driver-host（run_engine_loop/dispatch/BoundedSpawner/run_http_envelope/TimerRegistry/consume_effects 泛型 over Storage/Http/EventSink/Transport/Clock）→ native(1119→168 薄壳)+ffi 共用，parity by construction；W2 FFI 薄壳（专用线程 current_thread runtime · FfiEventSink 每-step 批 flush→on_domain_events_batch HX-C007 · FfiClock vtable 注入时间 · SharedWsClient::with_inbound_tick 入站 · destroy drop-sender→join）。BatchSink flush 钩子在 host 非 core
  ↳ 验证：cargo check 5 crate + clippy ffi 0 警告 · native 零回归(18 lib+stress 3+4+集成) · host 17 · ffi 12(含 e2e command→Emit→批回调 + lifecycle drain) · core 零改/零运行时(cargo tree 空) · 两轮验真 diff 0 blocking(W1 parity=sound/arch=sound-with-fixes · W2 parity=sound/ffi=sound-with-fixes，全 minor 已收) · 净 +2899/−1276 36 文件
  ↳ 留 TODO：helix_query 仍 honest stub（infra 就位：runtime handle+HostStorage；缺 query_name→ScanSpec 映射=IM 领域语义，硬编进 driver 会泄漏 schema 违反 runtime-agnostic §6 → 待 IM 暴露通用 query 路由）· DropOldest 误名（贯穿全仓既有术语·行为中性·defer）· native/ffi Clock 机制不对称（native ambient / ffi 注入，各满足确定性，需交叉回放再统一）
  ↳ ⚡ DRIFT-PRED：EXPECT ⑤「helix_query 走 spawn_blocking」实际偏离——保留 stub（IM schema 泄漏顾虑，W2 spec 已授权退路）。候选教训：FFI 同步读 query 的 name→Spec 映射属业务域，不能在 driver 层硬编（Storage trait 不泄漏 schema 铁律）→ 待 IM 暴露通用路由。已 type:surprise 落 logs。其余 6 断言全 held

🎯 EXPECT driver-ffi 转 C++-host 直连（cpp→rust→cpp · 砍 Dart 回调路径 + 生成 .hpp）@2026-06-18 | 赌注 中 | 可逆 是（worktree design/ffi-cpp-direct 未 merge）
  ↳ 可证伪预判：① 改动文件集**只在** crates/helix-driver-ffi/**（不碰 helix-core/helix-im）② bindingChoice 收敛为 cxx 或 cbindgen 二选一之确定值 ③ 砍掉 HelixCallbacks 的 Dart 形态（on_domain_event 作 Dart-Stream / send_ws_frame / send_http 逃生口定位）④ cargo check -p helix-driver-ffi + clippy 过、不新增 HX-C006 违反 ⑤ helix-core 仍零运行时（本次不碰 core）⑥ M3-F4 engine pump 仍留诚实 TODO（本次不实现整 pump）⑦ verify 无 blocking 才落地实现
  ↳ 偏离即记 DRIFT-PRED：若漏改进 core/im · binding 选择悬而未决 · 为求"重构"顺手实现整 pump · 或 verify 出 blocking 却仍强行落地 → 偏离

✅ DONE driver-ffi C++ 直连边界层重构 @2026-06-18 | commit cfe7a2a..ed22bbb（3 commit · branch design/ffi-cpp-direct 未 merge）| helix/design/ffi-cpp-direct
  ↳ 范围：砍 callbacks.rs + helix_on_ws_frame/http_response + FfiTransportMode::Callback（Dart-host 遗物）· 加 CppHostVtable(repr C, Send/Sync+SAFETY) · helix_create_with_network 末参换 vtable（复用不另造）· 加 helix_command(parking_lot::Mutex<Option<Sender>> + try_send/FFI_BUSY -4) · helix_query honest stub(FFI_UNIMPLEMENTED -5) · 统一 *mut HelixHandle · cbindgen.toml+build.rs 产出 helix.hpp(172 行：struct HelixHandle opaque + CppHostVtable + 6 导出) · 契约文档三件套(docs/design/cpp-direct/)
  ↳ 验证：cargo check/clippy/build 全 PASS · 两 lens 验真-diff sound-with-fixes **0 blocking** · 未碰 helix-core(零运行时)/helix-im/根黑名单 · 未臆造 core 类型(tick::Tick/AppCommand 用真源)
  ↳ 留 M3-F4（pump · honest TODO · 非本轮）：T1 pump 线程+runtime · T2 destroy join · T3 pump 主循环 · T4 Effect 分发(Emit→on_domain_events_batch HX-C007) · T5 helix_query 真实现(spawn_blocking 禁 block_on) · T6 HTTP status 镜像 native(任意 status→Ok(ReplyBytes)，只传输错→Err)
  ↳ ⚡ DRIFT-PRED：EXPECT 7 断言全 held，但**过程**偏离——预期「一个 workflow 搞定」实际走 3 轮 + 策略 pivot。根因：design-doc 对抗验证 FFI/core-touching 设计易臆造类型自信(R1/R2 共 9 blocking 多在 M3-F4 pump 内部)；改「实现优先 + 编译器/clippy 闸门 + 验真 diff」一轮收敛。候选约束(回放闸门待过 · 未达建卡三铁律阈值)：FFI 边界设计须显式划 M3-F4 不可触碰线 + HTTP status 镜像 native 是确定性不变量 + core 类型只 Read 真源。已 type:surprise 落 logs

⚠️ PARTIAL driver-ffi 转 C++ 直连 design round-1 @2026-06-18 | 卡在 verify(3 blocking + 5 major) → implement 正确 gated off（0 源码改动，branch design/ffi-cpp-direct 空）| 剩 round-2：答 2 架构问（WS 持有方 · 事件载荷 A/B）→ 修 design 全 blocker → 重验 → 落地 | workflow wf_7b1cc528-09c
  ↳ 设计已定 binding=cbindgen（cxx 排除：侵入式重写 + 双轨所有权 + panic 映射与 abort 冲突）。3 blocker：①helix_destroy+pump join use-after-free/死锁（tick_tx 副本致 recv 永不 None + destroy 与 on_ws_frame 并发）②cbindgen.toml+build.rs 双缺（.hpp 交付物无生成驱动）③handle 类型不一致（*mut c_void vs *mut HelixHandle → cbindgen 出两种指针型）。关键 major：PortReply 真型不符（Correlation/PortOutcome/PortError::Http(status)）+ HTTP 500 被当成功喂 core（破确定性）· try_send drop 入站帧破「慢而不丢」(HX-C008 cursor gap) · HX-C007 批处理只覆盖 Emit 漏 Send/Persist/Http/Timer

⚠️ PARTIAL driver-ffi 转 C++ 直连 round-2→3 @2026-06-18 | round-2(wf_269635e9 · 2 决策:Rust持WS+DomainEvent载荷)又 6 blocking 仍 gated off → round-3(wf_ae3af4bf)砍到「边界层 only」直接实现+编译器闸门，pump/query=诚实TODO | branch design/ffi-cpp-direct 仍空
  ↳ ⚡ DRIFT-PRED（候选教训，回放闸门待过）：连两轮 design-doc 验证栽在同一根因——**设计反复钻进 M3-F4 engine pump 内部**（step() 真签名/HTTP→PortOutcome 映射/block_on），而 pump 本就该留 TODO。真·契约纠错：①**HTTP status 必须镜像 native run_http**（完成响应任意 status→PortOutcome::Ok(ReplyBytes(json{status})) ，只传输错→Err via classify_port_error；4xx/5xx 判 Err = 跨驱动确定性 fork，破 GOAL §1/§4.5）——我 round-1 给的「status→Err」修法本身错，verify 纠正 ②PortError 真型 error.rs::PortError(String 承载) 非 tick.rs::PortError(u16) ③候选约束：「FFI 边界设计须显式划 M3-F4 pump 不可触碰线 + core 类型只 Read 真源不臆造」。design-doc 轮老在类型栽 → 改用「实现优先 + 编译器/clippy 当闸门 + 验真 diff」收敛

🎯 EXPECT 背压根治（Http/HttpFire 类型契约 + 统一 BoundedSpawner）@2026-06-17 | 只动 helix-core/{effect,engine}.rs · helix-driver-native/{engine_loop.rs,CLAUDE.md} · helix-im/tests/common.rs
  ↳ 可证伪预判：cargo check 四 crate 过 · helix-core 仍零运行时（cargo tree 空）· 不新增 HX-C001/C005 违反 · Http 默认翻 Block 不引入泵自死锁 · per-effect-kind ad-hoc 散落消除为单 BoundedSpawner（grep 无 live Semaphore）
  ↳ 偏离即记 DRIFT-PRED：若 core 出运行时依赖 / 泵 Http 分支出现 .await 阻塞 / 引入新 HX-C 破坏 → 偏离

✅ DONE 背压根治 @2026-06-17 ~15:35 | commit 0df4019..HEAD（core HttpFire + driver BoundedSpawner + im test + R-low 收敛）| helix/dev-zlc
  ↳ 范围：core Http/HttpFire 二分(仿 PersistFire) · driver 统一 BoundedSpawner(Persist N=1/Block · Http N=8/Block · HttpFire N=8/DropOldest) · 删旧 semaphore+atomic+泵顶 drain 三层 · 翻默认 Block 传导回源 · 跨模块公平性 tripwire 登记 · IM 三 Http 全保必达(PortReply load-bearing)
  ↳ 验证：cargo check 四 crate + 全测 PASS(driver lib 36 含 5 新测) · core 零运行时 · 设计 review 3 专家(deadlock/sans-io/general 全 True) · drift-review 🟡 轻(无破 HX-C001..C009/GOAL §1/§2/三层/sans-IO/单调 cursor)
  ↳ ⚡ DRIFT-PRED：EXPECT「不引入新 drift」与实际略偏——drift-review 揪出 R-low（均治理级非架构偏移，本批已收敛）：① HttpFire 零生产者空桶 YAGNI 越线 → option-C 文档化为「类型对称有意提前引入 + 拆除触发=第二模块接入」② engine_loop:308 recv 锁注释失真 → 诚实化 ③ §4.8 机制首跑被 SESSION:4 图例自污染(grep 未锚 §0) → workflow.js 改锚定版（本条即修）

✅ DONE a0595607 存储重构 review→修复闭环 @2026-06-15 16:38 | commit f44b78b..2b22736（4 commit）| B1/B2/M1/M2 + 遗留 #2/#3 全修 · gate 绿 · 已 push origin/main | helix/main
  ↳ 范围：B1 读写分离(1 写连接+4 只读池) · B2 NO_MUTEX · M1 query 不截断 · M2 SqlValue::Blob 无损 · #2 cursor SQL const 化(HX-C005) · #3 读写分离并发强证明测试 · 架构设计 sqlx→rusqlite 对齐(消 GOAL↔架构设计 矛盾) · ROADMAP 落遗留 · gate.sh 纳入 helix-driver-host
  ↳ 验证：gate.sh 全绿(三栖 check+clippy+test+零运行时+零业务+harness 索引) · helix-driver-host 测试绿 · 功能正确性 + E2/E5/engine_loop 5 不变量 + 读写分离并发 均验
  ↳ ⚠️ stress-verify verdict=REJECT（数字过：5000 行 4.3ms / 10k 78ms 不死锁 / drain 全落盘；但方法学弱：N=1 墙钟 / 回归阈值 10x 余量 / 并发未严格证明）→ 已落 ROADMAP P2（criterion 化 + 收紧阈值 + 并发强证明已补）。结论：功能正确且严格优于 a0595607，「承压不回归」为弱断言、非已严格证明
  ↳ 遗留（ROADMAP P2）：batch_upsert SQL 形状缓存 · FFI MAJ-4 spawn_blocking · M2 使 SqlValue::Blob 进 public-api 基线
  ↳ 2026-06-15 criterion 化已落（commit d1fb149）：弱断言→可签 median+95%CI——batch_upsert 5000 **5.16ms**[5.04,5.30]/969K rows/s · 39×100 6.08ms · monotonic cursor **28.5µs**[27.3,29.7]；回归判据 median>2x。stress「承压不回归」升级为可签硬数字
  ↳ 2026-06-15 criterion **全链路覆盖完成**（commit 1551c14）：engine 泵 `pump_roundtrip_1k` **40.2ms ≈ 24.9K roundtrips/s** · IM 乱序 `gate/backfill_5000` **8.0ms ≈ 624K events/s** · `send/echo_reconcile_1000` **2.39ms ≈ 418K/s**。性能基准从「仅 storage」→ storage+engine 泵+IM 乱序同步/对账 全有可签 median/CI；仅剩 bench 数字进 CI（待 runner）
