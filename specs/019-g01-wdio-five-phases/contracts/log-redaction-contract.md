# Console, Tauri And Report Redaction Contract

## Allowlist

| Plane | Allowed |
|---|---|
| Browser console | `runId`、`gate`、`persona`、`command`、`surface`、scope kind、`reqId`、`traceId`、`transactionId`、status/count/duration |
| Tauri/Helix | process start、pid、commit/pin、`reqId`、corr、effect kind、persist result、surface/revision/eventSeq/duration |
| Go/WS | `trackId`、`channelId`、`temporaryIdHash`、`postId`、`eventSeq`、event、commit/outbox status |
| Reporter | case/lane/result/timing、脱敏 correlation、artifact relative path/hash/size |

## Always Forbidden

`password`、`token`、`authorization`、`cookie`、session secret、完整请求/响应 body、原始消息正文、raw Frame/VM、数据库 dump、用户主目录任意文件。

## Enforcement

1. 采集前按字段 allowlist 投影，不先复制 raw 文件。
2. marker 只以受控测试 marker 或不可逆 `markerHash` 出现。
3. JSON/Markdown/JUnit/console/Tauri 附件全部运行同一 redaction validator。
4. 命中 forbidden key/value pattern 时报告生成失败，artifact 保留在受限临时目录并标记 `redactionVerdict=fail`。
5. 日志只读取当前 process start marker 到 run end 的 bounded window。
