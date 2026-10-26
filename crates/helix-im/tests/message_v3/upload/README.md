# MessageV3 Phase 1 Upload Gate 测试域

本目录覆盖 `MV3-G01c/G01d/G01e/G01g/G02d/G03c`。测试从 Host 已建立的 opaque
`mediaHandle` 或稳定 OSS 引用开始，Helix Core 不读取文件系统、不提取媒体元数据，也不持有
预签名 URL。

每个 Gate 必须通过 `ScenarioRunner -> ImModule::handle -> Effect` 验证 prepare、PUT
continuation、持久化屏障、选择性重试和最终 Post 投影。`MV3-G02d` 的 1 GiB+ 拒绝发生在 Host
ingress，要求零 Core Command、零 Persist、零 HTTP 和零 Event。

AUDIO 与 VIDEO 必须作为两个独立场景；多图必须同时断言逐文件绝对进度与按总字节加权的消息级
进度。参考 JSON 不是运行捕获，不得改写其证据等级。
