# Worktree Agent Task

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 路由结果

- Workflow: {{workflow}}
- Primary agents: {{primary_agents}}
- Reviewer agents: {{reviewer_agents}}
- Required references: {{required_references}}

## Worktree 规则

- 启动和每次 commit 前验证 `pwd` 包含指定 worktree 路径，分支等于指定 branch。
- 只能改 prompt 白名单路径；根 `SESSION.md`、`AGENTS.md`、`README.md` 默认禁改，除非主对话明确授权。
- 跨项目 gap 不阻塞：写 `NEED_<OTHER_PROJECT>_FIX.md`，继续推进可做部分。
- commit 前用精确 pathspec stage，并输出 `git diff --cached --name-only`。
- 报告包含 changed files、验证命令、是否触碰黑名单、是否有 NEED/RESOLVED 文件。
