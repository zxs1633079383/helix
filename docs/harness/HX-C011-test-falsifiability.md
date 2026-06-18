---
id: HX-C011
title: 压测/并发不变量测试必带证伪证明（破坏即 fail）· 禁墙钟下界断言（结构化/计数证不变量）
status: active
severity: major
scope: helix
created: 2026-06-18
source: 2026-06-18 三次同根（2026-06-15 stress-verify REJECT + storage 压测 2 false-green + P1-A 墙钟下界 flaky）
recurrence_count: 3
trigger_paths:
  - "crates/**/tests/**"
  - "crates/**/benches/**"
trigger_keywords:
  - "stress"
  - "bench"
  - "concurrency"
  - "Semaphore"
  - "monotonic"
  - "assert!(wall"
  - "elapsed"
  - "Duration"
  - "flaky"
related:
  - HX-C005
  - HX-C009
graduates_to:
  - ~/.claude/rules/common/testing.md
---

# HX-C011 — 测试可证伪性铁律（压测 / 并发 / 不变量）

## §1 触发场景
- 写**压测**（stress_*）/ **bench** / **并发**测试 / 任何「断言某不变量成立」的测试
- 声称「Semaphore 限流 / 分批」「cursor 防回退」「吞吐不回归」「写序保证」等命题的测试

## §2 背景（why）——三次同根
| # | 日期 | 现象 | 根因 |
|---|---|---|---|
| 1 | 2026-06-15 | stress-verify verdict=REJECT | 方法学弱：N=1 墙钟 / 回归阈值 10x 余量 / 并发未严格证明——「承压不回归」是弱断言非已证明 |
| 2 | 2026-06-18 | storage 压测 2 false-green（P1-C tautology · P3 无 assert） | P1-C 顺序写恒 100→删 MAX guard 照过=没证防回退；P3「<15%」只写注释无 assert + block_on 单 op 不跑并发 |
| 3 | 2026-06-18 | P1-A 墙钟下界断言 flaky（merge 后 gate 红） | `assert!(wall >= (batches-1)×base)` 假设分批串行，快读 pipeline 墙钟远小→在更快机器误 FAIL |

**核心**：测了 ≠ 证明了。**false-green（测了证不了）比没测更危险**——它给假信心。**timing 断言天生非确定**——墙钟受机器/调度/pipeline 影响，下界尤其 flaky。

## §3 Required / Forbidden
```
✅ Required：每个声称的不变量测试必须「可证伪」——
   临时破坏不变量（删 guard / 旁路限流 / 注入回退值）→ 测试必须 FAIL；还原 → PASS。
   提交压测/不变量测试时，在 commit body 或注释记录这条「破坏→FAIL→还原→PASS」证明。
   用结构化证据证不变量：状态断言（终态==max）/ 计数（max_concurrent<=N）/ 破坏即 fail，
   而非墙钟。
✅ 性能命题要么有真 assert（带可比 baseline），要么不声称（删判据，不留无 assert 的 eprintln）。
```
```
❌ Forbidden：
   - 墙钟「下界」断言（assert!(wall >= X) 证「分批/限流/串行」）——快路径 pipeline 即 flaky。
   - tautology：顺序写严格递增值后断言「终态==最大」——删掉防回退 guard 照样过 = 没证伪。
   - 无 assert 只 eprintln 的性能判据（注释写「<15%」「不回归」但代码不 fail）= false-green。
   - 把 false-green 当「已压测背书」写进 ledger / DONE（误导后续误以为命题已证）。
```
**墙钟「上界」断言可用**（assert!(wall < serial_upper) 证「非全串行」是安全方向，并发只会更快、bound 给足余量）；危险的是**下界**（断言「至少这么慢」）。

## §4 Verification
```bash
# 墙钟下界断言（危险）
grep -rn 'assert!(.*\(wall\|elapsed\).*>=' crates/*/tests crates/*/benches
# 无 assert 只 eprintln 的性能判据（false-green）——**函数级**扫描。
# ⚠️ 文件级 `grep -L 'assert!'` 对混合文件逃检：含 assert 的文件整体被 -L 排除，
#    块内 assert-less 测试永久漏检（见 §5 2026-06-18 实测）。必须按 fn 切块逐块判：
for f in crates/*/tests/*stress*.rs crates/*/benches/*.rs; do
  awk -v F="$f" '/^[[:space:]]*(pub )?(async )?fn /{if(n&&p&&!a)print F": "n; n=$0;p=0;a=0}
    /assert!/{a=1} /eprintln!.*(throughput|ops\/s|吞吐|回归)/{p=1}
    END{if(n&&p&&!a)print F": "n}' "$f"
done
# 压测/不变量测试应有「破坏→fail」证明（commit body 或注释）
```
> 审查准则：每个 stress/invariant 测试问一句「把不变量删了它会 fail 吗？」——不会 = false-green，打回。

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-15 | stress-verify | verdict=REJECT | N=1 墙钟 + 阈值松 + 并发未严格证明 |
| 2026-06-18 | storage 压测 verify-diff | P1-C tautology + P3 无 assert | 顺序写恒值 / 注释当判据 / block_on 单 op 不跑并发 |
| 2026-06-18 | daf44fe | P1-A 墙钟下界 flaky（finalize 侥幸过 / dev-zlc FAIL） | timing 下界假设分批串行，快读 pipeline 违反 |
| 2026-06-18 | 建卡当 diff drift-review | stress_storage_core 2 测无 assert（建卡同 diff 即违反）+ §4 文件级 grep -L 对混合文件逃检 | 下沉迁移漏补 assert；gate 文件级盲区 → 补 break→fail 断言 + gate 升函数级（本 commit） |

## §6 关联
- 兄弟：HX-C005（热路径 O(1)，压测对象）· HX-C009（预期对账：偏离不自产自判，false-green 是「自评通过实则证不了」的实例）
- 操作手册：`/helix-stress-verify` skill（承压验收必带数字 + 证伪）
- 上游：rules/common/testing.md（晋升候选）

## §7 历史与演进
- 2026-06-18 建卡：三次同根达三铁律阈值（≥3），过回放闸门（挡住三次历史 trap + 不误伤结构化好测如 `reads_proceed_while_write_holds_connection` / `monotonic_upsert_equal_value_is_noop` 这类「破坏即 fail」的真证伪测试）后确立。
