# Helix Agent Swarm Diagrams

这些 Mermaid `.mmd` 图是 Router 和 agent 派发前的轻量视觉索引。

- `project-domain-map.mmd`：项目总图，展示三层架构、模块、领域和 reviewer 闸门。
- `modules/*.mmd`：每个模块的 owned paths、对应领域、关键约束、默认 reviewer。
- `domains/*.mmd`：每个领域横跨哪些模块、依赖哪些源文件/文档、验收证据边界。

图谱只表达 agent 路由事实，不替代 `docs/GOAL.md`、`docs/ROADMAP.md` 或 `docs/harness/`。
