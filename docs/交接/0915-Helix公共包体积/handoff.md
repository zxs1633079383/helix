# Helix 公共包体积查询

- 状态：完成；用户明确范围为本项目的 Helix 公共包。
- 查询时间：2026-09-15 17:44（Asia/Shanghai）。
- 仓库：/System/Volumes/Data/workspace/angular/visual/helix
- 开局分支：reactor/MessageV3-helix
- 开局 HEAD：be863e4e42cab004294897e50c93a0173dd81b23
- 当前公共包共 4 个，最新版本均为 0.1.22，已实际下载并按 crates.io checksum 校验 SHA256。
- 来源：https://crates.io/api/v1/crates/{name} 与 https://static.crates.io/crates/{name}/{name}-{version}.crate。

| 包 | 压缩字节 | 解压文件字节 | 文件数 |
|---|---:|---:|---:|
| cses-helix-core | 37437 | 113332 | 28 |
| helix-im | 640416 | 2811569 | 252 |
| helix-driver-host | 165473 | 677172 | 65 |
| helix-driver-native | 38688 | 136925 | 18 |

- 总计：882014 bytes（861.34 KiB）；解压文件 3738998 bytes（3.57 MiB）。
- 每包历史版本 23 个，合计 92 个版本归档；API 元数据历史压缩体积总计 18800533 bytes（17.93 MiB），历史版本未全部下载。
- web、ffi、host-cli、perf 的 crates.io API 均返回 404。
- 大小仅含 Helix 自身源码包，不含第三方依赖或编译产物；解压体积按归档内普通文件长度求和，不计文件系统块开销。
- 证据与下载：artifacts/公共包体积-20260915/summary.json、各包 API JSON 与 .crate。
- Python urllib 下载遇 TLS EOF，改用 curl 官方静态地址成功；API download 路径曾返回 403。
- 保留用户现有已暂存分享稿删除；只提交本任务交接，不 push。
- 验证：4 个最新包下载与 SHA256 PASS；git diff --check PASS；无代码改动，无需 Cargo 验证。
- 下一步：无。
