# 发布 Helix 并更新客户端交接
更新时间：2026-09-10 Asia/Shanghai

## 目标与状态
- 用户授权重新构建、发布公共 crates.io，更新客户端依赖并通知；全部完成。
- 四公共包0.1.16 → 0.1.17；不push、不部署、不发布桌面安装包。

## 当前代码身份
- Helix：reactor/MessageV3-helix；发布提交17921c2136b1b02f3759cfe3a0be7a9e292b1e93。
- 注释Tag helix-v-0.1.17指向发布提交；其后仅本交接/SESSION收尾。
- 客户端：monorepo-message；9c817d7abecaea65af5cc9546c320f04770ddcc7。
- Helix原有artifacts/output未跟踪产物保留；客户端tracked clean。

## 已完成与验证
- publish_crates.py --plan与--publish成功：八包check、四包dry-run/上传/registry解析通过，RELEASE PASS。
- 八个workspace manifest与Cargo.lock形成完整版本事务；版本提交与Tag完成。
- IM全量152组：1519通过、旧注册总数断言1失败、11忽略；68916c57将137改为148并逐条断言11项分类命令，定向7/7通过。生产源码未变，其余结果复用。
- 合并阶段五crate check、core 22项+1 doc-test、WASM构建及55项聚焦结果复用同业务源码；1个core doc-test按原定义忽略。
- 客户端四项精确pin =0.1.17；本地被忽略Cargo.lock四包均来自crates.io 0.1.17。
- CSES_ENV=pre cargo check --manifest-path src-tauri/Cargo.toml通过（49.25s，388条warning）；git diff --check通过。

## 证据
- /tmp/helix-release-0910-publish.log
- /tmp/helix-release-0910-im-all-results.log
- /tmp/helix-release-0910-registry-test.log
- /tmp/helix-release-0910-client-check.log
- 合并测试身份见docs/交接/0910-合并冲突/handoff.md。

## 未验证与下一步
- 未重跑真实Go/PRE/UI链或桌面安装包验收，编译不替代业务验收。
- 本次无剩余发布/客户端升级工作；Git push等待用户授权。
