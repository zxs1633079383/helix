# Helix IM Fixture 索引（历史兼容）

> `RETIRED_HISTORICAL`：MessageV3 Gate 的当前 producer 与 canonical JSON 位于
> [`../message_v3/README.md`](../message_v3/README.md)；本目录不再承载业务 Gate 分母。

本目录当前只保留 `real_go_frames/` 下的真实 Go 协议帧样本，供 parser/replay 回归使用。
MessageV3 `{event,data}` 由 Rust Gate target 直接生成，不能把本目录样本或 consumer mirror
当成业务真源，也不得手工修改 JSON 来让测试通过。

## 证据边界

- 历史 Fixture 只证明当时 Helix producer 在确定性 sans-IO 剧本中产出了约定的 Frame；它们
  不是当前 MessageV3 Gate 的完成证据。
- `provenance.json`（若历史目录存在）记录 fixture inventory、producer、schema、surface、
  raw/semantic hash 与允许参数化的字段。
- Fixture/Mock 通过不等于 SQLite executor、真实 Go、WS、Tauri、DOM 或 Mobile 真机通过。
- 文件名中的 `.frame-v2.json` 是现有 artifact 命名，不代表内容一定是 schema 2；版本判断
  必须读取 JSON 的 `schemaVersion`。Timeline 可使用 schema 3，其他 Surface 仍可保持
  schema 2。

## 当前 MessageV3 Gate 入口

当前 30 个 Gate 的测试 target、producer 入口和四份 canonical JSON 约定见
[`../message_v3/README.md`](../message_v3/README.md) 及 Spec 047 migration manifest。
旧 `g*-wdio/`、`gs*-wdio/` 目录已从工作树删除，不应按本页历史表重新创建。

## 辅助 Fixture

| 目录 | 用途 |
|---|---|
| `real_go_frames/` | 真实 Go 协议帧样本，覆盖 connect、increment channel、sync 与忽略事件 |

该目录不是独立业务 Gate，不计入 Spec 047 的 30 Gate 分母。

## Producer 与更新入口

`real_go_frames/` 的样本由对应 parser/replay 测试通过 `include_str!` 读取；新增或更新真帧
时必须同步 `real_go_frames/README.md`、引用它的测试和来源说明。不得把历史 Frame JSON
改造成 MessageV3 producer，也不得用它替代真实 Go/SQLite/WS 证据。
