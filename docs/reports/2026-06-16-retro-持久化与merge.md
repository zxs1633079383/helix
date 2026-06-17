# 复盘 — 2026-06-16 B3 sync 真 wire + 678 持久化 + merge main（ADR-010）

> 范围：commit `8c69f25..c79dedd`（16 commit）。沉淀「踩了什么坑、怎么解、教训」，供后续 sync-v2 收尾参考。
> 配套真源：`docs/review/workflow-review-ledger.md`（drift 台账）· `docs/harness/HX-C008`（sync wire 契约）。

## 1. 遇到的问题 + 根因 + 解法

| # | 问题 | 根因 | 解法 / commit |
|---|---|---|---|
| 1 | 139 个 `sync response missing 'type'` | parser 按废弃抽象 `{type:...}` 解，现网真 wire 是 `{status,data:{entries:[SyncEntry]}}` | B3 真 wire 解析 + nextSeq 锚定（`6d83ad5`，HX-C008）|
| 2 | 持久化 schema 没建（只有 `_helix_monotonic`），真落消息会 `no such table` | Storage trait schema-agnostic，没人建业务表 | `IM_SCHEMA` 常量（helix-im）+ host-cli `execute_raw` 建表，对齐 678（`6d83ad5`）|
| 3 | deviceId 默认硬编码某台设备 | 图省事写死测试设备 id | 改本机派生（IOPlatformUUID/machine-id/MachineGuid），env 可覆盖（`6d83ad5`）|
| 4 | workflow 越权产物：持久化 workflow 自主衍生 eventbus 文档；review workflow 自主重构 docs | workflow agent 默认有写权限，没收口 | review/分析 workflow 加 **READ-ONLY 铁律**（`b411231`）；越权产物 revert |
| 5 | 自审盲区：持久化 workflow 自审漏 HX-C005 热路径回归 + channel_event_cursor 死表；merge 窄审漏 D1/D3；host-cli 修漏 bench caller(D2) | 同一 workflow 审自己有偏差；窄域 review 不覆盖全局 | 独立 drift-review panel（`97a33a8`/`fcf3222`）抓出后收敛 |
| 6 | 真 Go 冒烟抓到 `NativeHttp::new()` 运行时 panic（编译+单测都过） | merge 后 new() 退化为永远 Err，host-cli 仍调旧 API；测试不跑真 HTTP init | 改 `with_config(HostNetworkConfig)`（`6b2fd52`）；删活墓碑（`fcf3222`）|
| 7 | merge 架构冲突：main ADR-010 共享 client vs dev-zlc 直连 transport；main 薄壳 2 处 native P0 回归（send Binary / 入站不 emit `Tick::Inbound`）| 两分支架构独立演进，薄壳没覆盖 native 入站 push + Text 帧 | scout 覆盖核查 + 适配保住硬修（`dd61cd4`）：send→Text、`with_inbound_tick`、握手头灌 registry |
| 8 | ADR-008 编号撞车（dev-zlc=sync-v2 / main=shared-client）| 两分支独立加 ADR | merge 时 main 的改号 ADR-010（`dd61cd4`）|

## 2. 教训（可复用）

1. **适配现网真 wire 必核源代码**：cses-client（行为真源）+ Go server（协议真源）双源核对，不信设计文档抽象。用户 challenge「这是 cses 和 go 的语义吗」直接逼出 HX-C008 的 cursor-锚-nextSeq / 续拉-len≥500 两条护栏（逐字对齐 Go `sync.go` + cses `post.rs`）。
2. **动 I/O 层（HTTP/WS/Storage）必须真连回归**：编译 gate + 单测覆盖不了运行时构造/连接行为（NativeHttp::new panic、main 薄壳入站死）。真 Go :8065 直连冒烟是第三层、不可省。
3. **自审有系统性盲区**：workflow 审自己的产出会漏（HX-C005/D1/D3）；独立 panel + 真连回归各补一层。merge 边界（两架构融合）尤其要 **holistic drift 查**，不能只窄审改动文件。
4. **autonomous workflow 必须收口写权限**：review/分析类 workflow 一律 READ-ONLY；只有明确 fix/impl 类才给写权限 + 白名单。
5. **schema 设计阶段定形态**：Storage trait schema-agnostic（HX-C001），业务 DDL 当数据落 helix-im const，host 执行；cursor 真源单一（`_helix_monotonic`，channel_event_cursor 留 M2）。
6. **机制自验**：review-cadence（gap≥10 强制 drift-review）本会话首轮就自触发抓到真 drift，且机制自身缺陷（review agent 越权）也被同一轮暴露并修。

## 3. 留给 sync-v2 收尾的开放项

见 `crates/helix-im/docs/sync-v2-能力与缺口状态.md` §5 + ledger 待收敛 todo：
- **Phase C**：C1 echo→reconcile（`handle_echo` 死代码）· C2 B5 phantom + 离线消息落库 + readBits · C3 emit 6→24 + im:__bus__ 信封
- **Phase D**：golden fixtures 工具链 · UC-1 端到端真 Go smoke
- **HTTP/WS/Storage**：D1 FFI 句柄类型统一(HX-C006) · D3 native·FFI registry 对称化 · ping/退避重连 · B2 increment_end 冷启动自愈
