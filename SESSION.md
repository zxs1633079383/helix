# Helix SESSION

> 跨会话状态可查。§0 记三类固定前缀行（均可 grep · 详见 CLAUDE.md §4.8）：
> 开局 `🎯 EXPECT`（实质任务预判）· 收尾 `✅ DONE`（终态）· 偏离预期时 `⚡ DRIFT-PRED`（喂 §4.7 drift-review）。

## §0 终态记录

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

🔁 **dev-zlc 分叉监控（防再分叉 · 手动 cadence，durable cron 本环境不支持）**：新会话开局 + re-port 每个 commit 检查点，跑一次 `git rev-list --count feat/sync-v2-report-on-adr007..dev-zlc` 看落后多少；**落后 ≥5 或 dev-zlc 动了 `helix-driver-host/engine.rs`·`http_envelope.rs`·`helix-im` → 先 rebase 本分支到 dev-zlc 再续 re-port**，别让架构再岔。新会话也可自挂 session-only cron（`*/57 * * * *`）跑上面那条命令兜底。

✅ DONE sync-v2 全链真机闭环（在线+离线）@2026-06-18 | feat/sync-v2-full-replacement（参照源，未合并）：删库重拉离线 backfill 实证（Go `kind=events fromSeq=0`→message 落行）+ 在线发收 echo 回填。6 fix + member_count 全 commit、cargo/test 绿。**唯落到 dev-zlc 的 re-port 待做（见上续接）。**

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
