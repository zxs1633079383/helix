# Helix 提交规范（Commit / PR / Tag）

> **本仓所有贡献者（人 + Codex / Claude Code 等 agent）提交代码必须遵守本规范。**
> 来源：团队全局规范 `~/.claude/rules/common/git-workflow.md`，抽进仓内供任何拿到本仓的人/agent 就近遵守（Codex 读 `AGENTS.md`、CC 读 `CLAUDE.md`，两者均指向本文）。
> ⚠️ helix **有 `docs/harness/` 文件夹** → 结构化 body 的「Harness 更新」一节**必填**（不省略）。

---

## 一、Commit Message 规范

采用 **Conventional Commits**（大厂通用格式），**description / body 必须使用中文**：

```
<type>(<scope>): <中文描述>

<可选中文 body：解释 why，不解释 what>
```

- `<type>`：英文小写，固定集合（见下表）
- `<scope>`：英文小写，指明影响模块（如 `im`、`core`、`driver-native`、`crux`），可省略
- `<中文描述>`：简洁一句话，**≤ 50 字**，不加句号
- body（可选）：用中文解释**为什么这么改**、涉及的决策、踩过的坑；每行 ≤ 72 字符

### Type 取值

| type | 用途 |
|------|------|
| `feat` | 新功能 |
| `fix` | Bug 修复 |
| `refactor` | 重构（不改外部行为） |
| `perf` | 性能优化 |
| `docs` | 只改文档 |
| `test` | 只改测试 |
| `chore` | 构建 / 依赖 / 杂项 |
| `ci` | CI / CD 配置 |
| `style` | 纯格式（rustfmt / 空白），不涉及逻辑 |
| `revert` | 回滚历史 commit |

### 强制规则

- ❌ 禁止 `update xxx` / `modify xxx` / `wip` 这类没信息量的 message
- ❌ 禁止一次 commit 混多个 type（拆成多个 commit）
- ❌ 禁止 `--no-verify` 跳过 hook（除非用户明确授权）
- ❌ 禁止在 commit message 里写 "AI 生成" / attribution

---

## 二、Commit Body 结构化模板（强约束）

### 触发条件

**满足任一条**就必须用结构化 body（不只是一行 description）：

- 改 ≥ 3 个文件
- 净增/净减 ≥ 50 行
- 改动跨模块 / 跨 feature
- 涉及 schema / API / 协议变更
- 涉及 cutover / 重构 / 死代码清理
- 涉及 harness 文档更新或新增

**反例**（不需要结构化 body）：单文件 typo 修复、纯格式化、单测补缺、改 1 行配置。

### 5 段必填模板（顺序不可错、标题不可改名）

```
<type>(<scope>): <中文一行描述>

# 影响范围
- 列出受影响的文件 / 模块 / 子系统（精确到文件路径）
- 列出受影响的 IPC 表 / API 表 / DB schema / WS 协议 / 注册表条目

# 改动影响面
- 列出对调用方 / 运行时 / DB / 前端 / 后端的实际副作用
- 列出净增/净减行数（用 `git diff --stat` 精确数字）；保留 vs 待删的判定理由
- 列出本 commit 不动但下一 commit 要动的边界（写明 commit hash 或 Phase 编号）

# 功能改进/开发/新增
- 类型：feat / fix / refactor / perf / chore / docs / test / ci / style / revert
- 触发条件：为什么现在做（依赖谁完成 / 哪个 PR / 哪个用户决策）
- 后续：本 commit 的下游链路（如分批拆 commit / 后续 Phase 接力）

# Harness 更新（helix 有 docs/harness/ → 必填）
- 是否新增 harness card（三铁律：≥ 3 次同根因 / 用户明确 / Spec 拍板）
- 更新了哪些现有 HX-C{NNN}-*.md
- 更新了哪些索引（CLAUDE.md 表 / docs/harness/README.md）

# 验证
- cargo check（三栖/双端）/ cargo test / scripts/gate.sh / 端到端手工验证清单
- 关键耗时 + PASS/FAIL 状态
```

### Heredoc 写法（避免 markdown 转义，推荐）

```bash
git commit -m "$(cat <<'EOF'
<type>(<scope>): <中文描述>

# 影响范围
...
# 改动影响面
...
# 功能改进/开发/新增
...
# Harness 更新
...
# 验证
...
EOF
)"
```

### 实例参考（本仓真实 commit）

```
docs(crux): 沉淀 Crux 参照对照——边界三分法 + 还能取什么 8 向

# 影响范围
- 新增 docs/过会方案/helix-crux-参照对照.md
- docs/README.md / docs/过会方案/README.md（双索引登记）

# 改动影响面
- 三分法钉死 Crux 边界（✅做到 / 🚫故意不做渲染 / ⚠️真缺口大列表整份ship）
- DRY：构件映射详表不重复，cross-ref ROADMAP-full §4/§5/§6

# 功能改进/开发/新增
- 类型：docs
- 触发条件：用户澄清后要求沉淀结论
- 后续：Phase 6 接口草案时回查 §2

# Harness 更新
- 无新增 harness card

# 验证
- 新文档相对链接 + 全树断链重扫：0
```

### 强制规则补充

- ❌ 禁止结构化 commit 只写 description 一行不写 5 段 body
- ❌ 禁止 5 段顺序错位 / 段标题改名
- ❌ 禁止 5 段全空只写「TODO」（要么写实质内容要么删段）
- ❌ 禁止用户 challenge 后才补 body（应该 commit 时直接写）
- ✅ 多文件改动时影响范围用文件路径列表，不只写「多文件」
- ✅ 净增/净减行数用 `git diff --stat` 出的精确数字

### Pre-commit 流程

1. **不要先输出给用户看再 commit** —— body 直接写进 commit message（用户能 `git log -1` 看完整 body）
2. **不要因为「想给用户看」就阻塞 commit**
3. 用户说「pre-commit 之前定义 …」= 「commit body 自身要结构化」，不是「commit 前展示给我」

---

## 三、Pull Request 流程

1. 分析**完整 commit 历史**（不是只看最后一个 commit）；`git diff [base]...HEAD` 看全部变更
2. PR 标题沿用 Conventional Commits 格式 + 中文描述
3. PR body 用中文，至少含：**背景/动机（why）· 实现思路（how）· 测试计划（覆盖率+手工清单）· 风险与回滚方案**
4. 新分支 push 用 `-u` 建立 upstream

---

## 四、模块划分与 Tag 策略（多 Phase 大改才用，单 commit 小修不适用）

核心原则：「**一个 Phase = 一个模块 = 一个 tag**」——每完成一个独立可验证模块立即打 tag。

**模块切分铁律**：按业务边界切 / 按文件覆盖切 / 按风险分级切（dead-code 删除·契约变更·行为变更三类不混 commit）/ 每个模块自包含（单独 revert 不破坏构建）。

**Tag 命名**：
```
v<base>-phase<N>-<module-slug>     模块完成 tag（必打，带 message 说明覆盖 commit 范围 + 验证状态）
v<base>-rc<N>                       phase 间集成检查点（可选）
v<base>-final                       最后一个 phase 完成
```

**强制规则**：禁跨模块「杂烩 commit」；禁跳过 tag 进下一 Phase；tag 必带 message；推送用 `git push origin <tag>`（不用 `--tags` 一次推全）。

| 维度 | 关系 |
|------|------|
| 1 Sprint | 1 个 base 版本 |
| 1 Phase | 1 个或多个模块 tag |
| 1 模块 | ≥ 1 个 commit + 1 个 tag |
| 1 commit | 单一 type + 单一 scope |

---

> 本仓配套:`scripts/gate.sh`(提交闸门链) + `.git/hooks/pre-push`(push 前自动跑 gate)。提交前结构闸门见 [`ROADMAP.md` §三](./ROADMAP.md)。
