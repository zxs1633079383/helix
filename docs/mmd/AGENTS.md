# MessageV3 客户端交互图局部规则

## 作用域与入口

本文件适用于 `docs/mmd/` 及其所有子目录。它继承项目根
[`AGENTS.md`](../../AGENTS.md) 的通用约束；任务涉及客户端 Phase、Domain、Gate、Mermaid
时序图或 Helix 接缝时，先读本文件，再按需读取具体 Gate 文件，不把 92 个 Gate 的正文
复制回根指令。

本目录的阅读顺序固定为：

1. **L0：项目边界**：根 [`AGENTS.md`](../../AGENTS.md)，确认 Helix 三层架构、MessageV3
   事件边界和证据等级。
2. **L1：本地规则**：本文件，确认 92 Gate 分母、目录分层、真源和验证命令。
3. **L2：索引与分母**：[`README.md`](./README.md)，只读取目标 Phase、Domain 的表格和 Gate
   链接。
4. **L3：单 Gate 合同**：打开对应的 `PhaseN/<domain>/Gate-*.mmd`，先看首部真源注释、
   UI owner 和覆盖状态，再看交互步骤。
5. **L4：实现或运行态证据**：只有需要核对字段、命令、投影或验收证据时，才进入
   `cses-client` 的 Gate `contract.json`、UI/服务源码，以及 Helix `specs/047-*`、
   `crates/helix-im/docs/` 和对应测试/报告。

## 客户端 92 Gate 分母

这里的分母来自 PC 参考客户端
`/System/Volumes/Data/workspace/worktrees/sync-event-alignment/cses-client`，不是 Helix Spec 047 的
30 个 producer Gate。机器真源为：

- `specs/003-messagev3-gate-inventory/contracts/gate-registry.json`
- `scripts/message-v3/reference/phases/phase1.manifest.json`
- `scripts/message-v3/reference/phases/phase2.manifest.json`
- `scripts/message-v3/reference/phases/phase3.manifest.json`
- `scripts/message-v3/reference/phases/phase4.manifest.json`
- `scripts/message-v3/reference/phases/phase5.manifest.json`

| Phase | Domain | Gate 数 |
|---|---|---:|
| Phase1 | `channel`、`post`、`upload`、`draft`、`schedule`、`announcement` | 33 |
| Phase2 | `read`、`timeline`、`sync` | 20 |
| Phase3 | `channel`、`channel_member` | 15 |
| Phase4 | `pin`、`bookmark`、`search`、`announcement` | 12 |
| Phase5 | `chain`、`text_chain`、`category_chain`、`acceptance` | 12 |
| **合计** | **17 个 Phase 内领域目录** | **92** |

如果 registry、phase manifest、README 索引或实际文件的 Gate 数量、ID、Phase、Domain
不一致，立即停止并报告 `NOT_VERIFIED`；不得凭文件名或旧图猜测归属。

## 目录和文件合同

```text
docs/mmd/
├── AGENTS.md
├── README.md
├── 发送消息.mmd                 # 根参考图，不属于 92 Gate 分母
└── PhaseN/<domain>/Gate-*.mmd
```

每个客户端 Gate 只有一张图，路径必须满足：

```text
Phase1/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
Phase2/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
Phase3/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
Phase4/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
```

首部至少保留以下可追溯信息：

```mermaid
%% PhaseN / <domain> / MV3-Gxx
%% 客户端合同真源: <cses-client gate contract.json>
%% UI owners: <源码入口和行号>
```

## 图内交互边界

- 以 [发送消息参考图](./发送消息.mmd) 的逐跳粒度描述客户端与 Helix 的交互：Angular
  UI → Gate Port → Tauri Native / Flutter FFI / Browser Web → `Tick` / `helix-im` →
  SQLite、Go HTTP 或 WS → `PortReply`、`PersistOk`、raw event 或 Command Result。
- `helix-im` 负责业务事实、权限、排序、未读和恢复；Native、FFI、Web 只搬运原始
  `{event,data}` bytes，不在 shell 复制业务 reducer。
- HTTP 2xx、健康检查、静态注册和 Mock 结果都不是业务终态；需要明确标注
  `PersistOk`、权威 WS/PortReply、读回、`Effect::Emit {event,data}` 或 `partial/none`。
- `contract.json` 没有 Helix command、Effect、事件或真实入口时，图中使用 `NONE`、
  `partial`、`NOT_IMPLEMENTED` 等事实标记；禁止为了让流程看起来完整而发明 fallback。
- 只读 Gate 不得伪造 `PersistAtomic`、写入或 WS 权威回写；本地缓存读取必须和远端同步
  的事实边界分开。
- 图是交互合同和追踪入口，不把 Mermaid 能解析等同于 PC、Flutter、Web、Go、WS、
  SQLite 或双用户真实 Green。

## 修改工作流

### 新增或移动 Gate

1. 在 `cses-client` registry 和对应 phase manifest 中确认 Gate ID、Phase、Domain、名称。
2. 在唯一的 `PhaseN/<domain>/` 下新增或移动 `.mmd`，同步更新 [`README.md`](./README.md)
   的表格和链接。
3. 保留首部真源注释及现状覆盖标记；不复制旧 Phase 的路径作为兼容别名。
4. 检查 Gate ID 无重复、无遗漏，目录名与 manifest 一致。

### 修改单张图

1. 先读取首部的 UI owner、合同路径和覆盖状态，再回查对应客户端源码；需要 Helix
   结论时才进入 L4 真源。
2. 保持跨端顺序、`corr`/`req_id`、命令入口、Effect/事件、失败分支和负向事实的原意。
3. Mermaid 标签含括号、斜杠、冒号、箭头或其他标点时使用引号或合法转义；不要以
   静态文本替换真实跳转。
4. 文件使用 UTF-8，末尾保留一个换行，不产生空白行；不改动无关 Gate 或根目录参考图。

## 最小验证合同

文档结构变化必须运行：

```bash
# 1. 对照 cses-client phase manifest：92/92，Phase/Domain 全匹配
# 2. Mermaid CLI 逐图解析：所有 docs/mmd/**/*.mmd PASS
# 3. staged diff 检查
git diff --check -- docs/mmd
```

仅修改单图时至少解析目标图；修改目录、README、manifest 映射或 Gate 分母时必须重新
跑全量 92 张 Gate 图及所有参考图。该目录是文档任务，不因格式或索引变更自动扩大到 Cargo、Go、WDIO 或
真实 HTTP/WS 验收；若用户明确要求运行态验收，再按对应 Gate 的证据合同执行。

## 停止条件与交付报告

- 真源文件缺失、Gate ID 重复/遗漏、Phase/Domain 归属冲突或 UI owner 无法定位：停止，
  报告 `BLOCKED` 或 `NOT_VERIFIED`，不猜测。
- 发现旧图与当前客户端 manifest 冲突：以当前 manifest 为准，记录删除/迁移，不保留
  会造成重复分母的旧副本。
- 报告必须区分：图文件存在、Mermaid 语法通过、manifest 对账通过、以及真实客户端/Helix
  运行态是否验证；这些证据不可互相替代。
