# G04-G16 Native Aggregate Handoff

## 最终状态

**FULL GREEN**。017 的 35/35 tasks 已完成；最终 candidate 在一个 Native 拓扑内连续执行 19 个 Gate，`required fail=0`、`required skip=0`，未以单 UC 重编或旧 run 代替终局证据。

## Final Run

- Run ID：`mrc-g04-g16-final-a2f835f-1784780000`
- 账号：444 actor / 678 observer，companyId `64118eebd2b665246b7880eb`
- Gate：G-04、G-04a、G-05、G-06、G-07、G-08、G-09、G-10、G-10a、G-11、G-12、G-13、G-14、G-14a、G-15、G-15a、G-16、G-16b、G-16c
- 结果：19/19 `runtimeStatus=pass` 且 `canonicalStatus=green`
- 拓扑启动：1 次；suite wall-clock：186808ms
- 视口：actor/observer 均为 1440x900 final Native；Index-Dual 作为交互与视觉基准
- 证据根：`/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/.artifacts/mrc-g04-g16-final-a2f835f-1784780000`
- 19 个 `result.sha256` 合并摘要：`693a10120369536d0401ec031d3e1b18bdfdf18ecdc77e73d16c5cec65b8bfba`

## 修复闭环

- G14a 初始缺口：setup authority 只从 message row 取 eventSeq，修为允许 dialog row 的权威 eventSeq fallback；targeted rerun green。
- G04 初始缺口：temporary sent row 被误识别为 canonical final row，修正 optimistic identity；targeted rerun green。
- G08 final-pin 首跑缺口：频道增量重绘会吃掉观察者第一次“更多”点击；验收驱动改为重新选中目标频道、有限真实点击重试，并区分菜单未打开与业务状态错误；targeted 与最终全量均 green。
- Helix 全量回归额外修复：话题 kind 测试漂移、恢复压测 fixture authority、恢复 PersistAtomic 后重复 cursor 写、旧 UC-4 普通 Persist 合同。
- Go：频道设置权威 version、建群/话题/回执与 real-chain evidence 路径的 focused tests 全绿。
- Angular/Tauri：typed Frame whole-apply、双账号 lifecycle、DOM/ARIA/data-* 与 UI 状态回归全绿。
- 最终 pin：Helix `a2f835fbb4bee7dda62961c55c4fa9a931302d98`；cses-client 功能基线 `888f698bb`；Go `2251f95`。

## 验证

- `npx tsc -p tests/e2e/desktop/tsconfig.json --noEmit`：PASS
- `npm run test:desktop:uc`：3 suites / 15 tests PASS
- `cargo test -p helix-core`：PASS
- `cargo test -p helix-im`：全量 PASS（含 332 lib tests 与全部 integration tests）
- `cargo build --target wasm32-unknown-unknown -p helix-driver-web`：PASS
- `helix-core` runtime dependency grep：空
- `go test ./internal/api/post ./internal/app ./internal/realchain ./internal/entity/channel ./internal/entity/post`：PASS

## 各 UC Handoff

逐 UC 纵切、Gate 哈希与完成情况见 `specs/004-g04-revoke/handoff.md` 至 `specs/016-g16-channel-governance/handoff.md`。

## 未决项

无 Native required issue。移动 FFI 与 Web 平台专项验收不属于本 017 Native Gate，不在此伪称通过。
