# Dialog Projection Acceptance Report

日期：2026-07-09

## 结论

- 验收口径：只接受实际命令与文件证据；不把 worker 自报、mock-only、static-only、DB SKIP 写成 live green。
- 本轮总评：`PARTIAL`
- live 状态：`LIVE_E2E_NOT_RUN`

## 六个验收 case

| Case | Status | Evidence |
|---|---|---|
| normal | PARTIAL | `cargo test -p helix-im --test dialog_projection_test -- --nocapture` 通过，含 `visible_post_emits_render_ready_dialog_patch`；`projection-store.service.test.ts` 通过，含 `renders Helix update-by-post DialogPatch normal without deriving message rows`；2026-07-09 修复后重跑 exact npm focused gate PASS，2 suites / 19 tests passed；live 仍为 `LIVE_E2E_NOT_RUN` |
| mention | PARTIAL | `cargo test -p helix-im --test dialog_projection_test -- --nocapture` 通过，含 `mentions_and_urgent_posts_classify_and_bump_dialog_counts`；`projection-store.service.test.ts` 通过，含 `DialogPatch mention without deriving message rows`；2026-07-09 修复后重跑 exact npm focused gate PASS，2 suites / 19 tests passed；live 仍为 `LIVE_E2E_NOT_RUN` |
| urgent | PARTIAL | `cargo test -p helix-im --test dialog_projection_test -- --nocapture` 通过，含 `mentions_and_urgent_posts_classify_and_bump_dialog_counts`；`projection-store.service.test.ts` 通过，含 `DialogPatch urgent without deriving message rows`；2026-07-09 修复后重跑 exact npm focused gate PASS，2 suites / 19 tests passed；live 仍为 `LIVE_E2E_NOT_RUN` |
| own echo | PARTIAL | `cargo test -p helix-im --test dialog_projection_test -- --nocapture` 通过，含 `own_echo_updates_last_post_without_unread_delta`；`projection-store.service.test.ts` 通过，含 render-ready DialogPatch 覆盖且不本地推导 unread；2026-07-09 修复后重跑 exact npm focused gate PASS，2 suites / 19 tests passed；live 仍为 `LIVE_E2E_NOT_RUN` |
| sync visible/phantom | PARTIAL | `cargo test -p helix-im --test c3_sync_mutation_emits_test -- --nocapture` 通过，含 `new_visible_emits_received_and_channel_update`、`new_phantom_emits_nothing`、`sync_type1_sender_self_visible_exception_but_unread_zero`；Go focused gate 见命令 `G1`、`G2` 均通过；cses-client exact npm focused gate PASS，2 suites / 19 tests passed；live 仍为 `LIVE_E2E_NOT_RUN` |
| too_long | PARTIAL | `cargo test -p helix-im --test s6_sync_internal_test -- --nocapture` 通过，含 `s6_too_long_drops_channel_messages_and_emits_reload`；`cargo test -p helix-im proactive_too_long_requests_latest_post_reload_e6 -- --nocapture` 通过；`projection-store.service.test.ts` 通过，含 `marks sync too_long as a Helix-owned waiting state without clearing local UI messages`；`node scripts/contract-gate.mjs --json /tmp/mobile-contract-review.json --md /tmp/mobile-contract-review.md` 与 `./scripts/run-gtest-js-spec.sh` 均通过，确认 mobile 对 `readLastSeq/msgCount` 只做 optional 透传、不伪造 `0`；cses-client exact npm focused gate PASS，2 suites / 19 tests passed；live 仍为 `LIVE_E2E_NOT_RUN` |

## Non-live gate 执行结果

| Repo | Command | Result | Notes |
|---|---|---|---|
| Helix | `cargo test -p helix-im --test dialog_projection_test -- --nocapture` | PASS | normal / mention / urgent / own echo 主链路 |
| Helix | `cargo test -p helix-im --test projection_schema_test -- --nocapture` | PASS | `im_channel_update_by_post_is_render_ready_dialog_patch` 等 35 项 |
| Helix | `cargo test -p helix-im --test c3_sync_mutation_emits_test -- --nocapture` | PASS | visible / phantom / read / revoke / edit |
| Helix | `cargo test -p helix-im --test s6_sync_internal_test -- --nocapture` | PASS | too_long drop-and-reload |
| Helix | `cargo test -p helix-im --test http6_sync_reply_unwrap_test -- --nocapture` | PASS | sync reply unwrap |
| Helix | `cargo test -p helix-im test_gate_buffer_capped_proactive_too_long_e6 -- --nocapture` | PASS | proactive gate overflow |
| Helix | `cargo test -p helix-im proactive_too_long_requests_latest_post_reload_e6 -- --nocapture` | PASS | proactive too_long 触发 reload |
| Helix | `cargo test -p helix-driver-host --test storage_conformance guarded_bump_extra_counts_share_the_same_guard_s3 -- --nocapture` | PASS | count guard |
| Helix | `cargo test -p helix-im --test s6_sync_internal_test s6_too_long_preserves_messages_and_emits_reload -- --nocapture` | 待 2026-07-14 统一复验 | too_long 保留旧消息并覆盖式重拉 |
| Helix | `cargo test -p helix-driver-ffi --test mobile_contract_test mobile_im_send_alias -- --nocapture` | PASS | Task 2 alias |
| Helix | `cargo test -p helix-driver-ffi --test mobile_contract_test -- --nocapture` | PASS | Task 3 capabilities / version |
| Helix | `cargo check -p helix-driver-ffi` | PASS | FFI 编译 |
| Go | `G1` | PASS | post fields / messages map / read / too_long mock |
| Go | `go test ./internal/app/sync -run TestPostToSyncMessage_KeepsHelixProjectionFields -count=1` | PASS | sync payload 字段保留 |
| Go | `G2` | PASS | viewers 收窄 |
| Go | `if [ -n "$CSES_IM_TEST_DSN" ]; then ... else echo 'SKIP: CSES_IM_TEST_DSN is not set'; fi` | SKIP | 当前环境 `CSES_IM_TEST_DSN` 为空，不能宣称 live DB green |
| mobile | `node scripts/contract-gate.mjs --json /tmp/mobile-contract-review.json --md /tmp/mobile-contract-review.md` | PASS | `coverage/mobile-uc-contract-gate.{json,md}` 已按最新规则重生成；`dialogPatch.readLastSeq/msgCount` 不再列为 producer required，改为 optional-only 透传 |
| mobile | `./scripts/run-gtest-js-spec.sh` | PASS | 77 项里 76 PASS，1 项 `MOBILE_GTEST_REAL_CORE=1` 环境约定 SKIP |
| mobile | `make real-chain-manifest` | PASS | `46/46 cases + 6 dialog projection cases` |
| mobile | `make gtest-js-spec` | CONCERN | 失败原因为 `HELIX_REPO is pinned ... but shared checkout is at ...`，是 pinned rev/checkout 环境问题，不是本轮业务 case 红 |
| cses-client | `npm test -- --runTestsByPath src/pages/message-v4/__tests__/projection-store.service.test.ts src/pages/message-v4/__tests__/projection-handler.registry.test.ts --runInBand` | PASS | 2026-07-09 修复后重跑通过，2 suites / 19 tests passed |
| cses-client | `/System/Volumes/Data/workspace/angular/cses-client/node_modules/@playwright/test/node_modules/.bin/playwright test -c tests/e2e message-helix-hard-cut.spec.ts` | PASS | 14/14 通过 |
| cses-client | `npm run test:message-v4:real-chain-scope` | FAIL | package script 指向 worktree 内不存在的 `node_modules/.../playwright`，报 `No such file or directory` |
| cses-client | `/System/Volumes/Data/workspace/angular/cses-client/node_modules/@playwright/test/node_modules/.bin/playwright test -c tests/e2e message-real-chain-scope.spec.ts message-real-chain-evidence.spec.ts` | PASS | 4/4 通过，确认无 green 越界与证据矩阵约束 |
| All relevant repos | `git diff --check` | PASS | Helix / Go / mobile / cses-client 当前 worktree 均通过 |

Go 命令详情：

```bash
# G1
go test ./internal/app -run 'TestPublishPostEvent_IncludesHelixProjectionFields|TestBuildMessagesMap_NilPostSkip|TestSyncChannels_(TooLongMock|ReadEventNoMessageBodyMock)$' -count=1

# G2
go test ./internal/ws -run 'TestResolveViewers_(AllBranches|PointerStringSlice|PqStringArray|UnknownShape)$' -count=1
```

## Live E2E 状态

`LIVE_E2E_NOT_RUN`

原因只按当前仓库与环境实证记录：

- 未发现可直接执行的“两用户 dialog projection 真链路”现成命令；`package.json` 里存在 `message:real-chain:collect` 与 `test:message-v4:real-chain-scope`，但前者是收集/文档脚本，后者是静态 scope/evidence gate，不是 live 两端验收命令。
- 当前环境未见 auth / cookie / backend 相关变量：`env | rg '^(BASE_URL|CSES|MESSAGE|PLAYWRIGHT|AUTH|COOKIE|HELIX|GO)_'` 无命中。
- 当前验收没有明确 fixture channel / 双账号准备步骤 / backend URL 绑定方式。

因此本轮只能给出 non-live `PARTIAL`/FAIL/SKIP 证据，不能把任何 case 升格为 live green。

## 剩余 concerns

1. `npm run test:message-v4:real-chain-scope` 的 package script 路径在 worktree 下失效；等价 absolute Playwright 命令可通过。
2. Go DB integration 仍是 `SKIP`，因为 `CSES_IM_TEST_DSN` 未设置。
3. mobile contract gate 已转绿，但 `make gtest-js-spec` 仍受 pinned Helix rev 与共享 checkout 不一致影响，只能把该项记为 `CONCERN`。
