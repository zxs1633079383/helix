# RICH 上传实时进度（2026-10-09）

## 实现与身份

- 基线 `helix-v-0.1.48` / `5f4c8ce3`，分支 `codex/rich-upload-progress`。
- 客户端默认把单视频、多图和混合附件发送为 `RICH + props.files`。旧实现对 RichImage/RichVideo 禁用字节进度，且进度注册仅接受 File；前端能显示进度并不代表此路径会产生进度。
- RichImage/RichVideo 现在使用真实上传字节的 5% 档位。更新目标文件 `uploadProgress`，按文件大小计算消息汇总；仅 PersistOk 后更新内存快照并触发原 timeline 回读。
- 同消息的进度、完成、失败写入共用持久化回执屏障，避免并发附件整列 props 写入互相覆盖。排队进度合并为最新值，终态优先，忽略重复/迟到进度。不同消息互不阻塞。
- File 保留原顶层标量进度，TemplateImage 保留禁用策略。没有新增公开 Port、命令、schema 或平台依赖，没有改变版本。
- 字节回调通过 correlation 索引定位；仅被接受的 5% 档位制作最多 9 个附件的有界快照，不扫描消息历史。写入复用现有私有字段清理。

## 定向验证

- `cargo +stable check -p helix-im -p helix-driver-host`：通过，40.35 秒。
- `cargo +stable test -p helix-im --test message_v3_mv3_g01c --test message_v3_mv3_g01d --test message_v3_mv3_g01e --test message_v3_mv3_g02d --test message_v3_mv3_g03c`：50 项通过。包含单视频、并发加权/合并、落库失败不污染兄弟状态、完成/失败等待进度提交、迟到进度与重复 ACK。
- `cargo +stable test -p helix-driver-host --test media_pipeline_e2e`：1 项通过。实际 SharedFileUploader、本地 HTTP、真实 SQLite、9 个混合图/视频目标；同时验证持久化中间进度和内核真实 `im:timeline:ready/update` 中的逐文件中间进度，保留普通 FILE 回归。
- 集成测试的 Java/OSS/Go 是本地合成端点，媒体是按 MIME 分类的合成字节；证明传输、提交、事件组合，不证明真实服务端、解码或 Tauri UI 验收。测试补齐持久频道 user/team 归属并等待 timeline ready，符合 0.1.48 的 RenderScope 过滤条件。
- 当前机器固定 1.95 工具链缺少 cargo，Helix 检查使用已安装 stable 1.97.1；未修改工具链声明。
- 日志在客户端 `work/helix-progress-{upload,host,check}.log`，编译产物使用本工作树独立 target。

## 未通过和未覆盖

### 实机追查：动态窗口标识（2026-10-09 晚）

用户在修复版发送 14,070,953 字节的 RICH MP4，OSS 于 22:51:19 至 22:51:32 上传成功，posts/create 返回 200，但界面没有中间进度。真实查询使用 `mv3-window-...`，普通 send 不传 window token；进度提交后的 `unwrap_or("latest")` 却把 latest 当作实际窗口名，`is_attached` 不匹配而静默跳过刷新。

把宿主组合测试的窗口名从 latest 改为动态值后，原实现出现中间进度事件断言失败（`helix-progress-window-red.log`）。修复为：显式 token 保持原重试窗口约束；无 token 时复用现有频道活动窗口回读入口。同一宿主管线分别测试固定/动态窗口，2 项通过（`helix-progress-window-green.log`）。前一次只有 latest 的组合测试覆盖不足，不应作为生产默认窗口链路通过的证据。本次未改窗口附着、分页或锚点机制。

后续真实界面进度仍需用户重试确认；此次日志已证实该文件可以上传，不能继续把它归为上传失败。

首轮扩大到 post 的 G01A / G01F 时各遇到 1 条 HTTP admission readback 数量断言失败（期望 1，实际 0）；已停止无关范围扩张，未修改它们的断言或用例。它们不走本次媒体进度路由，疑似同类频道归属 fixture 缺失，尚未做基线 A/B，不能将其标记已解决。日志 `helix-progress-related.log`、`helix-progress-focused.log`。

未执行真实 Tauri 发送、真实 OSS/Java/Go 验收、三端完整验收或发布。此前用户报告的“视频无法上传”尚缺具体失败日志，不等同于本次进度缺失；30 秒上传期限和 64 MiB 限制未改变。

## 客户端接入与人工回归

客户端 manifest 仍固定注册表 0.1.48；本地源码不会自动进入已运行进程。开发接入使用客户端既有 `HELIX_ROOT` 覆盖机制指向本工作树，重新编译并启动；不得将旧 sibling 的 0.1.26 接入，也不修改注册表缓存。

1. 在授权测试会话发送低于 64 MiB 的单 MP4，观察多个中间百分比及最终发送。
2. 图片和视频混发，检查逐文件进度与总字节加权汇总；一个附件失败时不能误发整条消息。
3. 断网失败、恢复、重试，检查从初值重新推进；旧请求的迟到进度不能覆盖新状态。
4. 记录客户端构建身份、文件大小、时间、错误和截图；实机证据补齐前保持验收待执行。

版本与发布由用户决定，本任务不提版、不 push、不发布。Harness 无新增卡片或索引，回归沉淀在现有 Gate 测试和宿主组合测试。
