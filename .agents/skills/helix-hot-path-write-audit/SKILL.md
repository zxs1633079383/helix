---
name: helix-hot-path-write-audit
description: Helix Rust 热路径代码 Review 闸门。编辑或审查 helix-im、core Effect、native/host/FFI driver、SQLite StorageOp、消息投影和 push_msg 时使用；检查 HX-C005 的零分配 O(1)、重复 clone、整条 wire JSON 序列化、数组 read-modify-write、逐条 FFI/Effect、阻塞 I/O 泄漏到 core。出现 clone、serde_json、StorageOp、INSERT、UPDATE、Effect、落库、热路径或写性能时自动触发。
---

# helix-hot-path-write-audit

仓库适用范围：`crates/helix-core/`、`crates/helix-im/`、`crates/helix-driver-host/`、`crates/helix-driver-native/`、`crates/helix-driver-ffi/`。除通用 lint 外，Review 必须同时对照根 `AGENTS.md` 的 HX-C001、HX-C005、HX-C007；禁止把 runtime、I/O 或并发引入 core。

审计所有热路径写入代码（每个事件至少触发一次的写入路径），以根 `AGENTS.md` 的
HX-C005、HX-C007 和本 Skill 的五条铁律为准。

The rule was created after the 2026-05-17 incident: cses-client `push_msg` did
`payload.props.clone()` + `serde_json::to_string(&wire_message)` per push — 2 深拷贝 + 1 JSON
per event, on a hot path.

## When to invoke

Invoke automatically when **any** of these is true:

- The user mentions: `hot-path` / 热路径 / 写性能 / 落库 / push_msg / event handler
- Edit/Write touches a `*.rs` / `*.go` / `*.sql` file containing one of:
  - `serde_json::to_string(&payload|&wire|&msg|&event)`
  - `json.Marshal(payload|wire|msg|event)` 在写库前
  - `INSERT INTO messages` / `INSERT INTO events` / 类似 hot-path INSERT
  - `UPDATE ... SET props =` / `UPDATE ... SET queue =` / array-column 整列回写
  - 同一函数体内出现 ≥ 2 次 `.clone()`
  - schema 里 `props TEXT DEFAULT '[]'` / `queue TEXT DEFAULT '[]'`
- The user runs `/hot-path-audit <file>` or asks "审计一下这段写库代码".

Do NOT invoke for cold-path code (config writes, 用户名修改, 1Hz 以下事件) — see
"Exceptions" below.

## Procedure

### 1. Run the audit script

```bash
bash .agents/skills/helix-hot-path-write-audit/audit.sh <file>
```

The script inspects `*.rs` / `*.go` / `*.sql` and emits violations as:

```
[hot-path-violation] <file>:<line>: <pattern-name>
  matched: <line content>
  rule:    <one-line explanation>
  fix:     <suggested action>
```

Exit code `1` → at least one violation; `0` → clean.

Patterns detected:
1. **wire-json-roundtrip** — `serde_json::to_string(&payload|&wire|&msg|&event)` / `json.Marshal(...)` 把整条 wire 对象塞单列。
2. **multi-clone-per-fn** — 同一函数体 `.clone()` ≥ 2 次（O(N) 深拷贝堆叠）。
3. **rmw-array-column** — `UPDATE ... SET (props|queue|list|history|mentions) = ...` 整列回写 JSON 数组。
4. **json-array-default-column** — schema 里 `props TEXT DEFAULT '[]'` / `queue TEXT DEFAULT '[]'` 等 JSON 数组列。
5. **go-marshal-before-insert** — Go `json.Marshal(payload|wire|msg|event)` 紧邻 INSERT / Exec。

### 2. Apply the 5 铁律

铁律总览：

1. 写复杂度 = O(1)（单条 INSERT / UPDATE WHERE PK / DELETE WHERE PK）
2. 绝不 read-modify-write 整个 JSON 列
3. 协议级稳定字段拆独立列，禁止 `serde_json::to_string(整条 wire)` 塞 props
4. 累计 / 队列 / 多值字段 normalized 到独立表（联合主键），append = O(1) INSERT
5. 索引按读路径设计（覆盖 WHERE 列组合）

反模式词典以本 Skill 的“Anti-patterns to flag during review”表和根 `AGENTS.md`
的 HX-C005 为准。

### 3. Exceptions (must satisfy ALL 3)

仅当下列三条**同时**成立，才能保留 JSON 数组 / blob 列：

- 写入频率 ≤ 1 Hz（配置项 / 用户名 / 偶发设置）
- 单条载荷 ≤ 128 bytes
- 代码注释里写明事件率上限 + 凭据（"why this is cold-path"）

任一条不满足 → normalized schema，独立列 + 独立子表。

### 4. Pre-write checklist

设计新 handler / 新 schema 前过一遍：

- [ ] 每个写操作复杂度是 O(1)？
- [ ] 没有 JSON 数组列存累计 / 队列 / 列表 / 历史？
- [ ] 没有 `serde_json::to_string(entire_wire_obj)` 塞单列？
- [ ] 协议级稳定字段都已拆独立列？
- [ ] 已设计读路径索引？
- [ ] 当前 PR 内已写 migration？（不留 TODO）

任一项 NO → 停下重新设计 schema，不要"先 ship 再优化"。

### 5. 修复对照

示例 + 修复对照见 `.agents/skills/helix-hot-path-write-audit/example-violation.md`
（基于 2026-05-17 cses-client push_msg 实战案例）。

## Anti-patterns to flag during review

简表：

| 词 / 短语 | 危害 |
|---|---|
| 「先存 JSON 再说 / props 兜底」 | 字段无索引、热路径 O(N) RMW |
| 「暂不动 schema」/「下一阶段再拆」 | 永远没有下一阶段 |
| 「数组列存累计队列」 | 单条 push = 整列读+parse+写 = O(N²) |
| 「无损往返 = to_string(整条 obj) 塞单列」 | 无法索引 + 序列化 CPU 浪费 + schema 演进卡死 |
| 「等性能问题暴露再拆」 | 暴露时已积数据，迁移代价指数级 |

听到用户或 PR 描述里出现这些短语 → 立即触发本 skill。

## Edge cases

- **SQL file**: 只检测 `json-array-default-column` 模式。
- **Test file** (`_test.go` / `tests/`): 默认仍审计，但允许通过 `# hot-path-audit: ignore` 行内注释豁免。
- **非 hot-path 但匹配模式**: 例如 startup 一次性 marshal config — 在该行加 `// hot-path-audit: cold (≤1Hz, 64B, config write)` 注释跳过。

## Don't

- 不要因为"看起来像 hot-path"就 panic 重写整张表 — 先确认事件率，再确认 schema 必要性。
- 不要把这个 skill 当成 silver bullet：它是 lint 不是 review，最终判断要靠人 + 上下文。
- 不要在 cold-path 强行 normalized（增加 query JOIN 反而拖慢读）。
