# Helix Project Router

Router 把用户任务、影响路径、关键词和约束订阅转成 agent 队形。它不替代项目 AGENTS/CLAUDE/GOAL；它只决定应该把哪些上下文交给哪些 worker/reviewer。

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 路由顺序

1. 先按路径命中 `.agents/manifests/modules.json` 的模块 agent。
2. 再按路径/关键词命中 `.agents/manifests/domains.json` 的领域 agent。
3. 再按 `.agents/manifests/constraint-subscriptions.json` 追加 required references 和 reviewer。
4. 跨 core/driver/IM/FFI/trace/WS/Storage 的任务升级为 `adversarial-workflow`。
5. 如果只命中一个低风险边界，使用 `classify-and-act`；多边界默认 `route-and-synthesize`。

## 必守边界

- 不允许把 IM schema、wire 字段、temporaryId 推进 `helix-core`。
- 不允许把重连/HTTP/WS/SQLite 业务实现塞进 `helix-core`。
- 不允许把 `helix-driver-wasm` 当作当前 roadmap/CI 交付目标。
- 不允许把 HTTP 200、WS、projection/storage、DOM 证据混为一个绿色结论。
- 不允许 worker 覆盖 crate 级 `AGENTS.md`/`CLAUDE.md` 镜像文件；若确需修改，必须成对同步。

## 输出契约

Router output 必须包含：

- `workflow`
- `primary_agents`
- `reviewer_agents`
- `required_references`
- `notes`
