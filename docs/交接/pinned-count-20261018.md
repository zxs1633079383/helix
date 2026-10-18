# 置顶列表与频道摘要一致性修复

## 已确认根因

- 真实 PRE 双账号隔离 Tauri：成员先进入测试群，topCount=0、权威列表0；另一账号置顶成功后，成员权威列表1而topCount仍0。客户端证据：`cses-client/artifacts/pinned-interaction-20261017/count-member/{before,after-pin}.json`。
- `post_pin` 使置顶集合失效，但原权威列表落库只写 channel_pinned_projection，未更新 channel.top_count。
- Go 单频道转换读取成员历史 TopCount，FillDefaultValue 又将未知补零；可把旧计数作为显式权威覆盖。Go 修复单独交付。
- program_full 的通用 collect_present 保护已经保留缺失 topCount，本次只补回归，不修改该逻辑。

## 当前修复

- 置顶权威列表与 channel.top_count 原子保存，提交后按频道 ID 读回计数，发布现有 im:channel:update。
- 业务拒绝/非数组结果不进入缓存；持久化失败不发布摘要成功。
- 本地读取同时核对缓存列表与摘要；不一致返回 cache miss，复用权威读取修复旧数据。正常缓存不额外请求 HTTP。
- 复杂度：每次读取固定两次主键查询；远端列表更新固定一次原子写和一次主键读回。无成员遍历、无逐成员 HTTP/DB 请求。

## 验证与交付边界

见 `artifacts/pinned-count-20261018/` 和客户端同任务候选/桌面证据。定向公开 Tick、缓存和稀疏摘要回归；真实 PRE 新候选双实例通过：置顶后1/1，原生按钮取消后0/0；仅隔离实例注入缓存1/摘要0后，原生切群重进恢复1/1，独立SQLite读回确认。候选SHA256 `3ce8823424166bc09631848d382a79fa8d378dda4fc82729eb62c68a05122308`。

定向测试：read_relay_test 41、channel_write_program_test 14、pinned_projection 4，共59项通过；cargo check -p helix-im通过（既有9条warning）。
用户批准 patch 后，四个公开 crate 已发布 crates.io 0.1.74；版本提交 bb57fe75，注释标签 helix-v-0.1.74，未 push。发布脚本八包 check、四包 dry-run及可解析验证通过；core测试、WebAssembly构建通过；IM库492项通过，全量在G13a三个既有失败停止，与旧registry基线一致。日志见 artifacts/pinned-count-20261018/release/。

CSES 精确依赖及锁文件已更新到0.1.74，registry桌面验收见客户端回归记录。Go已按用户本批覆盖率例外提交8af7b1b7，尚未部署；不能声称新Go实时帧已验收或用户旧实例已升级。
