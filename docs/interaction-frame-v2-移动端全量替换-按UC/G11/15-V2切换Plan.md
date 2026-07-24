# G11 V2 切换 Plan

1. 对账 [PC G11](../../interaction-frame-v2-全量替换-按UC/G11/README.md) 与 Mobile command/binding。
2. 为 `message.timeline` attach、snapshot、action、receipt、ack/resync 写 RED 测试。
3. 接通 im.posts.locate -> MobileImSdk -> CoreBridge。
4. 让 `im:interaction:frame` 成为本 Gate 唯一 renderer 输入。
5. 首轮纯 V2 收集 HTTP/WS/PG/SQLite/Frame/UI 证据。
6. 删除 V1 producer+consumer 和 raw reducer，再用新 candidate 二次验收。
