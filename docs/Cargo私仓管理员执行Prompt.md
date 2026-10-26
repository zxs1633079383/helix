# Cargo 私仓管理员执行 Prompt

> 安全说明：本文只记录账号名 `admin`，不记录任何真实密码或 Token。当前密码已经通过聊天
> 明文传递，应先轮换，再通过密码管理器或其他安全通道提供。Cargo 发布优先使用 Artifactory
> Identity Token，不把账号密码写进仓库、命令历史或 CI 日志。

以下内容可直接转发给 Artifactory 管理员执行。

---

## 可直接转发的 Prompt

```text
请在现有 JFrog Artifactory 中为 Helix Rust crates 新建一个独立的 Cargo Local Repository，
并配置现有 admin 账号具备发布和下载能力。

一、当前环境与账号约束

- Artifactory Base URL：http://maven.wehanyu.com:8082/artifactory/
- 当前服务版本：Artifactory 7.84.16
- 现有 Maven 仓库：java3
- 当前统一访问主体：repoUser=admin
- repoPassword 由负责人通过安全渠道单独提供，不得写入聊天、Git、脚本、截图或验收日志。
- Cargo 客户端优先使用 admin 对应的 Artifactory Identity Token；不要直接把明文密码提交到
  ~/.cargo/config.toml、项目配置或 CI 文件。

请先确认 admin 是否为 Artifactory 内置超级管理员：

1. 如果 admin 是内置超级管理员，它已经拥有全局管理、发布、下载和删除权限，普通
   Permission Target 无法把它收敛成“仅发布和下载”。本次可以复用它完成建仓和首次验证，
   但请在交付结果中明确标记该风险。
2. 如果 admin 只是同名普通账号，请仅授予 helix-cargo-local 的 Read、Deploy/Write 和
   Cargo publish 所需权限；不要授予其他仓库、Delete、Manage Repository 或全局 Admin。
3. 不得因为现有账号方便而把 repoPassword 固化到仓库或多人机器。后续应能够平滑切换为
   独立的 helix-cargo-publisher 服务账号和只读账号，而不改变 Cargo 仓库地址。

二、新建 Cargo Local Repository

请创建：

- Repository Key：helix-cargo-local
- Package Type：Cargo
- Repository Type：Local
- Index Type：Sparse HTTP Index
- 启用 sparse index support
- 禁止匿名发布
- 是否允许匿名下载按公司安全策略决定；默认不开放匿名下载
- 发布版本按不可变制品管理，禁止覆盖同名同版本

现有 java3 是 Maven 仓库，请不要修改、删除或复用 java3，也不要将裸 .crate 文件上传到
java3 后冒充 Cargo registry。

预期 Cargo Sparse Index 地址：

http://maven.wehanyu.com:8082/artifactory/api/cargo/helix-cargo-local/index/

请确认以下地址返回 HTTP 200 和合法 JSON：

http://maven.wehanyu.com:8082/artifactory/api/cargo/helix-cargo-local/index/config.json

如果管理页面没有 Cargo Package Type，或者当前许可证不支持 Cargo Local Repository，请
立即停止，不要用 Maven/Generic 仓库替代。请返回：

1. 当前 Artifactory edition 和 license；
2. 是否支持 Cargo Local Repository；
3. 缺失的许可证或功能；
4. 可执行的升级或替代方案。

三、配置 admin 的发布与下载能力

请创建 Permission Target：helix-cargo-local-permission，范围仅包含：

- helix-cargo-local

若 admin 是普通账号，至少授予：

- Read/Download
- Deploy/Write
- Cargo publish 所需权限
- Cargo yank 所需权限仅在公司允许时授予

默认不得授予：

- 其他 Maven/Cargo 仓库权限
- Delete Repository
- Manage Repository
- 全局 Admin

若 admin 是内置超级管理员，请不要声称上述 Permission Target 已限制它的权限；请如实报告
“admin 仍为全局超级管理员，本次仅复用其身份进行发布和下载”。

四、为 Cargo 提供认证凭据

请为 admin 生成或确认可用于 Cargo registry 的 Artifactory Identity Token，并通过安全渠道
单独交付。不要在工单、聊天或验收输出中粘贴真实 Token。

开发人员配置：

# ~/.cargo/config.toml

[registry]
global-credential-providers = ["cargo:token"]

[registries.wehanyu]
index = "sparse+http://maven.wehanyu.com:8082/artifactory/api/cargo/helix-cargo-local/index/"

登录命令：

cargo login --registry wehanyu

上述命令提示输入 Token 时，粘贴 admin 对应的 Artifactory Identity Token。不要在命令参数、
项目文件或 shell 脚本中写 repoPassword。

发布示例：

cargo publish --registry wehanyu -p cses-helix-core

客户端下载示例：

[dependencies]
cses-helix-core = { version = "=0.1.1", registry = "wehanyu" }

五、验收要求

请完成真实读写验收，不能只反馈“仓库已创建”：

1. config.json 返回 HTTP 200 和合法 Cargo registry 配置；
2. Cargo 能通过 sparse index 连接 helix-cargo-local；
3. admin 对应 Identity Token 能执行 cargo publish；
4. 同一凭据能执行 cargo info、cargo search 或依赖解析下载；
5. 无凭据或错误凭据不能 publish；
6. 发布唯一版本的测试 crate，例如 helix-registry-smoke@0.0.1；
7. 在另一台机器或清理本地缓存后仍能解析并下载该测试 crate；
8. 所有日志必须脱敏，不得出现 repoPassword、Identity Token 或 Authorization 请求头；
9. 不得删除、覆盖或影响 java3 中的任何 Maven 制品。

建议验证命令：

cargo search --registry wehanyu helix-registry-smoke
cargo info --registry wehanyu helix-registry-smoke@0.0.1

如果测试 crate 需要清理，请先说明 Artifactory 私仓的清理策略并取得确认；不要未经确认删除
仓库或其他制品。

六、HTTPS 要求

当前地址是明文 HTTP。账号密码或 Identity Token 通过 HTTP 传输存在泄露风险。请同时给出：

- HTTPS 接入地址或上线计划；
- TLS 证书和反向代理方案；
- HTTPS 上线后最终的 Cargo sparse index URL。

生产发布前应切换 HTTPS；若暂时只能使用 HTTP，必须在交付结果中标记安全风险和预计整改时间。

七、交付结果

请返回以下脱敏信息：

- Repository Key
- Package Type
- Sparse Index URL
- config.json HTTP 状态
- admin 是否为内置超级管理员
- admin 实际权限范围
- Permission Target 名称
- Identity Token 的申请/轮换方式，不包含真实 Token
- 测试 crate 名称和版本
- publish 测试结果
- download/resolve 测试结果
- HTTPS 当前状态和整改计划
- 如有阻塞，提供准确错误信息和许可证限制
```

## 内部使用提醒

- 当前可以按要求复用 `admin` 完成首次建仓、发布和下载验证。
- 若 `admin` 是内置超级管理员，“授予一定权限”并不会降低它的全局权限；长期共享该账号等于
  共享整个 Artifactory 管理权限。
- 正式使用时，建议新增 `helix-cargo-publisher` 和 `helix-cargo-reader`，分别授予发布和只读权限。
- 当前 Helix 发布脚本仍固定使用 crates.io；私仓建成后，需要为
  `scripts/publish_crates.py` 增加统一的 `--registry wehanyu` 支持，才能继续使用一键四包发布。
