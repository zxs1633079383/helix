# Source Discovery: G04/G04a 撤回与离线撤回

**Feature**: `specs/004-g04-revoke`
**Source Lock**: `e8ff8f0725e8793f23fe86b70c4f017a867565eb`
**Worktree State**: `dirty; pre-existing user changes preserved; this feature remains spec-only`

## Facts *(mandatory)*

- **F-001**: G04 在线 action 与 G04a 离线 type=3 replay 共用 normalize/persist/frame，离线链不生成 HTTP Effect。 | **Evidence**: `crates/helix-im/docs/interaction-v2/完整时序和数据/01-G04-G07-撤回转发回应与加急完整时序和数据.md:21`
- **F-002**: G04-G16c 除 viewReceipt 外的 live ActionDispatch 尚未完成，存在 VM 或 outbound builder 不等于 capability 已上线。 | **Evidence**: `crates/helix-im/docs/interaction-v2/完整时序和数据/README.md:26`
- **F-003**: 所有完整业务 Frame 必须晚于权威输入的 PersistAtomic/PersistOk，HTTP 200/SUCCESS 不能建立 mutation 完成。 | **Evidence**: `crates/helix-im/docs/interaction-v2/完整时序和数据/README.md:123`
- **F-004**: 完整 MessageVm 与各 Surface VM 的必填字段由 InteractionFrame V2 公共清单约束，客户端不得消费 partial row。 | **Evidence**: `crates/helix-im/docs/interaction-v2/完整时序和数据/README.md:105`

## Unknowns

- **U-001 RESOLVED**: 当前实现状态不得由规格推断为 live。 | **Resolution**: 在 spec 的 CURRENT/TARGET/GAP 表中显式标注，并由 focused test 与 017 runtime evidence 分别晋级。
- **U-002 RESOLVED**: 跨仓修改不能在同一脏目录里互相覆盖。 | **Resolution**: 实施前为 Helix、Go、cses-client 分别记录 revision/dirty state，并使用独立 worktree 或在当前 owner 仓保留已有修改后最小编辑。

## Decisions

- **D-001**: G04 以 UC/Gate 为唯一交付颗粒，不和其他 Gate 合并实现。
- **D-002**: ActionBinding、Tick、Persist、Surface/VM、Frame、Angular/UI 均需独立 task 与 validator。
- **D-003**: 单 Gate 只形成 focused readiness；Native、双账号、远端 PG、本地 SQLite 与视觉证据集中到 017。

