# Helix SESSION

> 跨会话状态可查。§0 记三类固定前缀行（均可 grep · 详见 CLAUDE.md §4.8）：
> 开局 `🎯 EXPECT`（实质任务预判）· 收尾 `✅ DONE`（终态）· 偏离预期时 `⚡ DRIFT-PRED`（喂 §4.7 drift-review）。

## §0 终态记录

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
