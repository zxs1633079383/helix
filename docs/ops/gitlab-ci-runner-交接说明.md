# GitLab CI Runner 交接说明（helix 仓 → 运维）

> 2026-06-15 · 目标：给 helix 仓（`http://git.yundiz.com/cses/go/helix.git`）激活 GitLab CI。
> 仓库已自带 [`.gitlab-ci.yml`](../../.gitlab-ci.yml)（两个 job：`gate` 跑编译/clippy/测试/护栏闸门，`api-surface` 跑公开 API 面 diff）。
> **CI 本身能跑通的前提是「有一个能编译 Rust 的 runner」——这部分 yml 管不了，需要运维配。**

---

## 一、需要运维做的 4 件事（docker executor 推荐）

| # | 事项 | 说明 | 完成后回填给开发 |
|---|---|---|---|
| 1 | **注册 GitLab Runner** | 给项目 `cses/go/helix`（或 group `cses/go`）注册一个 runner，executor = **docker**。项目 Settings → CI/CD → Runners 拿 registration token | runner 是否设了 **tag**（如 `rust`/`docker`）——有的话告诉我，我在 yml job 加 `tags:` |
| 2 | **`rust:1.95.0` 镜像可拉** | yml 用官方 `rust:1.95.0`（含 rustup，自动对齐 `rust-toolchain.toml`）。**若内网拉不到 Docker Hub**：把 `rust:1.95.0` 同步进咱们私有 registry（Harbor 等） | 私有 registry 里的**完整镜像地址**（如 `harbor.yundiz.com/library/rust:1.95.0`），我改 yml 的 `image:` |
| 3 | **crates.io 可达 / 或给私有镜像** | `gate` 要编译整个依赖树，需要拉 crate 源。**若内网访问不了 crates.io**：提供一个私有 cargo **sparse 镜像**（registry index 地址） | 私有 crates **sparse index 地址**（如 `sparse+http://mirror.yundiz.com/crates/index/`），我配进 `.cargo/config.toml` |
| 4 | **runner 资源** | Rust 编译吃资源。建议 **≥4 vCPU / ≥8 GB RAM / ≥20 GB 磁盘**（target + cargo cache）。docker executor 建议开 cache volume，否则每次冷编译慢 | 确认资源规格即可 |

---

## 二、验证（运维配完 → 开发 push 触发）

1. 运维完成上面 1–4，把回填项给开发。
2. 开发按回填改 `.gitlab-ci.yml`（镜像地址 / crates 镜像 / tags）后 `git push`。
3. push 自动触发 pipeline：
   - **`gate` job 应全绿**（三栖 cargo check + clippy + test + 零运行时/零业务/harness 索引闸门）；
   - **`api-surface` job** 首次生成 API 基线、`allow_failure: true` 不挡（基线 commit 入库后才真正卡红）。
4. `gate` 绿 = CI 激活成功。

---

## 三、备选：shell executor（runner 那台机器已装好 Rust 1.95.0）

如果不走 docker、而是在一台**已装 rust 1.95.0 + wasm32 target** 的机器上跑 shell executor：
- 不需要镜像（第 2 项免）；
- 开发把 yml 里 `image:` 和 `before_script` 的 rustup 行删掉，`gate` job 只留 `script: scripts/gate.sh`；
- crates.io 可达性（第 3 项）仍需要。

> 告诉开发是 **docker 还是 shell**，开发把 `.gitlab-ci.yml` 改成对应版。

---

## 四、一句话给运维
> 帮 `cses/go/helix` 注册一个 **docker executor 的 GitLab Runner**；确保它能拉 **`rust:1.95.0`** 镜像、能访问 **crates.io（或给私有 cargo 镜像）**；建议 4C8G20G。拉不到的镜像/源同步进私有 registry 后把地址发我，我改 yml。配完我 push 验证 `gate` job 绿即激活成功。
