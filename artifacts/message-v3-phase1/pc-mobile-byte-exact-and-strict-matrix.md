# Phase 1 PC/Mobile Byte-exact 与 Strict Matrix 报告

日期：2026-07-31

## 分母与结果

- Phase 1 registry：33 个 Gate target，30 个真实接入 Gate + 3 个 Gap。
- 从 registry 提取 33 个 `test_target` 后单轮运行：33/33 target 通过，exit 0。
- byte-exact 覆盖包含草稿 PC/Mobile 结果、公告 PC/Mobile 请求、回执 sender/receiver 等合同断言。
- Strict Matrix、command API、artifact schema 与 Phase 1 registry 在当前 deterministic/contract 层一致。

## 判定

- Rust deterministic/contract：`PASS`。
- PC/Mobile 合同字节一致：`PASS`。
- 真实 PC 与 Mobile 消费者运行时一致：`UNVERIFIED`。
- dual-user、Native、canonical：`UNVERIFIED`，canonical 仍为 `0/30`。

本报告不以 Rust target 的 byte-exact 断言替代真实设备、真实 Frame 或双用户证据。
