//! MV3-G05l｜零 IM 写入工具栏命令（**纯负向 Gate**）。
//!
//! 参考包（线型权威）：`specs/003-messagev3-gate-inventory/gates/post/mv3-g05l/`
//! （`status: NOT_APPLICABLE`、`rust_entry_inputs: []`、`allowed_effect_tags: []`）。
//!
//! 「多选 / 复制 / 提醒 / 协同 / 头脑风暴」五个工具栏动作是**纯本地 UI 交互**：
//! `multiple` 只翻转选择态，其余四个只弹一句占位提示。参考包 `negativeAssertions` 的核心
//! 规则是：**helix 接入时不得为这 5 个命令分配 command** —— 不得把纯本地交互升级成 Port 调用。
//!
//! ## 缺陷 14 修正：从「名字形」改成「注册表补集形」
//!
//! 本文件此前用一张手工枚举的 15 条候选名清单（`im_<action>` / `im_<action>_post` /
//! `im_post_<action>` 三种拼法）做负向断言。那是**猜测清单**：真出现
//! `im_toolbar_remind` / `im_remind_users` / `im_message_copy` 这类命名时，四条测试会
//! **静默通过** —— 负向 Gate 挡不住新增的违规，等于没有。
//!
//! 现在改为对**命令注册表取补集**：
//!
//! - **可枚举真源**（自动增长，无需维护）：
//!   `outbound::outbound_command_names()`（inventory 注册表）
//!   ∪ `query::query_command_names()`
//!   ∪ `client_api::COMMAND_ALIASES` 的 public / core 两侧
//!   ∪ `client_api::capabilities_json()["commands"]`（对三端 driver 广播的公开命令面）。
//! - **Phase 1 声明的合法业务命令集**：从 33 个 Gate 参考包的
//!   `inbound-contract.json::rust_commands[].name` 机读得到（线型权威，不在本文件手抄）。
//! - **补集** = 注册表 − 声明集。补集里的任何命令都不是 Phase 1 声明过的业务入口；
//!   其中任何一条**可归因于工具栏动作**的命令都判违规。
//!
//! 新增命令自动进入扫描域，不需要有人记得来更新猜测清单。
//!
//! ## 三道防空转措施
//!
//! 1. **探测器自检**（`detector_flags_every_toolbar_attributable_command_name`）：
//!    直接拿复核给出的四个真实反例喂归因谓词，必须全部判违规。探测器一旦退化回
//!    「精确名字匹配」，这条先红。
//! 2. **补集非空守卫**：若声明集读取逻辑坏掉（读成整张注册表），补集会塌成空集，
//!    后面的「补集内零违规」就变成 vacuous truth。故显式断言补集规模下界。
//! 3. **差分回放**（`toolbar_action_ticks_are_transparent_to_the_next_real_command`）：
//!    不再只数「四项为 0」（那条任何随机字符串都能通过，只是 dispatcher 未知命令 no-op
//!    的通用行为），而是断言「插入工具栏 Tick 前后，后续真命令产出的 Effect 全序列
//!    逐字节相同」——真正针对「零 IM 状态面写入」这条 Gate 语义。对照组（插入一条真
//!    inbound 消息）必须让序列产生差异，证明差分装置有鉴别力。
//!
//! 证伪实验（已实测，见任务报告）：
//! - 把 `is_toolbar_attributable` 换回「候选名精确匹配」→ 措施 1 红。
//! - 在 `client_api/capabilities.rs` 的 `commands` 里加一行 `"im_toolbar_remind"`
//!   → `no_command_surface_entry_is_attributable_to_a_toolbar_action` 红。
//! - 把差分回放的 `assert_eq!` 改成比较不同长度的前缀 → 措施 3 红。

use helix_core::effect::Effect;
use helix_core::tick::AppCommand;
use helix_core::Tick;
use std::borrow::Cow;
use std::collections::BTreeSet;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, inbound_event, Harness};

/// 构造一条**运行时命名**的命令 Tick。
///
/// `common::command_json` 只接受 `&'static str`，而本 Gate 要遍历一批运行时拼出的候选名，
/// 故在此直接构造 `Cow::Owned` 的 `AppCommand`（`Tick::Command` 的公开形态）。
fn owned_command(name: &str, payload: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&payload).expect("payload must serialize");
    Tick::Command(AppCommand {
        name: Cow::Owned(name.to_string()),
        payload: bytes::Bytes::from(bytes),
    })
}

const ACTOR_ID: &str = "user-actor-444";
const COMPANY_ID: &str = "company-phase1";
const NOW_MS: u64 = 1_700_000_000_000;

/// 参考包 `inbound.entryPoints` 的五个工具栏命令 type，逐字对齐
/// `src/pages/message-v3/service/toolbar-command/commands/*.command.ts`。
const TOOLBAR_ACTIONS: [&str; 5] = ["multiple", "copy", "remind", "synergy", "brainstorm"];

// ─── 归因谓词（替代旧的手工候选名清单）────────────────────────────────────────

/// 工具栏语义**词根**表：命中即判定该命令名可归因于某个工具栏动作。
///
/// 词根而非全名，是为了覆盖真实命名变体而不是三种固定拼法：
/// `remind` 覆盖 `im_toolbar_remind` / `im_remind_users` / `im_reminder_send`；
/// `multi` 覆盖 `multiple` / `multi` / `multiselect`；`select` 覆盖 `selection` / `selected`。
///
/// 每个词根都直接来自参考包 `contract.json` 对这 5 个动作的语义描述
/// （多选 = multiple/select、复制 = copy/clipboard、提醒 = remind、协同 = synergy/collab、
/// 头脑风暴 = brainstorm），外加命令面自身的 `toolbar` 前缀词。
const TOOLBAR_STEMS: [&str; 8] = [
    "multi",      // multiple / multiselect / multi
    "select",     // select / selection / selected
    "copy",       // copy / copying
    "clipboard",  // 复制的另一常见命名
    "remind",     // remind / reminder / reminders
    "synergy",    // synergy
    "collab",     // 协同的另一常见命名：collaborate / collaboration
    "brainstorm", // brainstorm
];

/// 该命令名是否可归因于工具栏动作。
///
/// 按 `_` 切段后逐段前缀匹配词根。用「段」而不是「整串 contains」，避免把
/// `im_channel_*` 这类含子串的正常命令误判；用「前缀」而不是「全等」，
/// 覆盖 `reminder` / `selection` / `multiple` 这类词形变化。
///
/// `toolbar` 单独处理：它不是动作名而是命令面归属标记，任何带该段的命令都必须
/// 由本 Gate 复核（Phase 1 未声明任何工具栏命令）。
fn is_toolbar_attributable(name: &str) -> bool {
    name.split('_').any(|segment| {
        segment == "toolbar" || TOOLBAR_STEMS.iter().any(|stem| segment.starts_with(stem))
    })
}

/// 复核意见「缺陷 14」给出的实证反例：真实形态的新增违规命令名。
///
/// 旧的 15 条猜测清单对这四个名字全部漏判（四条负向测试静默通过）。
const TOOLBAR_VIOLATION_EXEMPLARS: [&str; 5] = [
    "im_toolbar_remind",
    "im_remind_users",
    "im_message_copy",
    "im_multi_select_posts",
    "im_post_brainstorm_create",
];

// ─── 可枚举真源 ①：helix 命令注册表 ─────────────────────────────────────────

/// helix 对外可达的**全部**命令名闭集（自动增长的真源，不含任何手工枚举）。
///
/// 四个面并集，因为「给工具栏动作分配 command」可以从任一面发生：
/// outbound inventory 注册表 / query 表 / 公开别名表（public 与 core 两侧）/
/// capabilities 广播面。
fn command_surface() -> BTreeSet<String> {
    let mut surface: BTreeSet<String> = BTreeSet::new();

    for name in helix_im::outbound::outbound_command_names() {
        surface.insert(name.to_string());
    }
    for name in helix_im::query::query_command_names() {
        surface.insert((*name).to_string());
    }
    for alias in helix_im::client_api::COMMAND_ALIASES {
        surface.insert(alias.public.to_string());
        surface.insert(alias.core.to_string());
    }

    let caps: serde_json::Value =
        serde_json::from_slice(&helix_im::client_api::capabilities_json())
            .expect("capabilities must be valid JSON");
    for value in caps["commands"]
        .as_array()
        .expect("capabilities.commands must be an array")
    {
        surface.insert(
            value
                .as_str()
                .expect("capabilities.commands entries must be strings")
                .to_string(),
        );
    }

    surface
}

// ─── 可枚举真源 ②：Phase 1 声明的合法业务命令集 ─────────────────────────────

/// 从 33 个 Gate 参考包机读 Phase 1 声明的合法业务命令集。
///
/// 真源 = `specs/047-im-event-gate-inventory/contracts/message-v3-phase1-send.registry.json`
/// 的 `entries[].reference_path` → visual 仓
/// `gates/<path>/inbound-contract.json::rust_commands[].name`。
/// **不在本文件手抄命令名**：Gate 包改了，这里自动跟着改。
fn phase1_declared_commands() -> BTreeSet<String> {
    let registry = phase1_reference::read_json(&phase1_reference::registry_path());
    let root = phase1_reference::reference_root();
    let entries = registry["entries"]
        .as_array()
        .expect("registry.entries must be an array");

    let mut declared = BTreeSet::new();
    for entry in entries {
        let relative = entry["reference_path"]
            .as_str()
            .expect("registry entry must declare reference_path");
        let contract =
            phase1_reference::read_json(&root.join(relative).join("inbound-contract.json"));
        let Some(commands) = contract["rust_commands"].as_array() else {
            continue;
        };
        for command in commands {
            let name = command["name"]
                .as_str()
                .expect("rust_commands entries must declare a string name");
            declared.insert(name.to_string());
        }
    }
    declared
}

/// 四项零计数（参考包「专项断言」①Command ②Http ③Storage ④Emit）。
#[derive(Debug, Default, PartialEq, Eq)]
struct SideEffectCounts {
    /// ① 下游 IM Command：本模块把命令翻译成的 `Send`（WS 出帧）与 `Request`。
    command: usize,
    /// ② HTTP：`Http` / `HttpFire`。
    http: usize,
    /// ③ Storage 写入：`Persist` / `PersistAtomic` / `PersistFire`。
    storage: usize,
    /// ④ 业务事件：`Emit`。
    event: usize,
}

impl SideEffectCounts {
    fn of(effects: &[Effect]) -> Self {
        let mut counts = Self::default();
        for effect in effects {
            match effect {
                Effect::Send { .. } | Effect::Request { .. } => counts.command += 1,
                Effect::Http { .. } | Effect::HttpFire { .. } => counts.http += 1,
                Effect::Persist { .. }
                | Effect::PersistAtomic { .. }
                | Effect::PersistFire { .. } => counts.storage += 1,
                Effect::Emit { .. } => counts.event += 1,
                // Timer / UploadFile 不属于本 Gate 的四项计数，但也一并纳入总量守卫。
                _ => {}
            }
        }
        counts
    }

    fn is_all_zero(&self) -> bool {
        *self == Self::default()
    }
}

/// 建空场景：注册一个频道、注入身份，但**不喂任何 Tick**。
fn empty_scenario() -> Harness {
    // Keep both the prelude channel and the negative-control channel as real
    // persisted scopes; the control must not rely on an unknown channel.
    Harness::new_with_identity(&[1, 2], ACTOR_ID, COMPANY_ID, "", "", "")
}

/// 五个工具栏动作在 helix 命令边界上可能出现的**标识符**形态。
///
/// 这批名字只用于「标识符不可分发」这条断言（工具栏动作的原始 type 本身不得成为命令），
/// **不再**承担负向覆盖职责——覆盖由 `is_toolbar_attributable` 对注册表取补集承担。
fn toolbar_identifiers() -> Vec<String> {
    let mut names = Vec::with_capacity(TOOLBAR_ACTIONS.len() * 2);
    for action in TOOLBAR_ACTIONS {
        names.push(action.to_string());
        names.push(format!("im_{action}"));
    }
    names
}

/// 证明 MV3-G05l 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G05l", "post/mv3-g05l");
}

// ─── 措施 1：探测器自检（缺陷 14 的复现 + 回归锁）───────────────────────────

/// 归因谓词必须判出复核给出的四个真实反例；且不得误伤现有合法命令。
///
/// 这是本 Gate 的**探测器自检**。旧实现（手工候选名精确匹配）在这条上全数漏判——
/// 把 `is_toolbar_attributable` 换回精确匹配，本测试立刻红。
#[test]
fn detector_flags_every_toolbar_attributable_command_name() {
    for exemplar in TOOLBAR_VIOLATION_EXEMPLARS {
        assert!(
            is_toolbar_attributable(exemplar),
            "MV3-G05l 负向探测器漏判 `{exemplar}`：工具栏动作被升级成 helix command 却没被判违规"
        );
    }

    // 反向：真实业务命令不得被误伤，否则本 Gate 会退化成噪音而被人整体禁用。
    for legitimate in [
        "im_send_message",
        "im_urgent_post",
        "im_template_received",
        "im_channel_member_snapshot",
        "im_query_messages_by_channel",
        "im_relay_messages",
    ] {
        assert!(
            !is_toolbar_attributable(legitimate),
            "MV3-G05l 负向探测器误伤合法命令 `{legitimate}`"
        );
    }
}

// ─── 措施 2：注册表补集扫描（本 Gate 主断言）─────────────────────────────────

/// 两个可枚举真源都必须读得出东西，且补集非空。
///
/// 这是补集扫描的**防空转前置**：声明集读成整张注册表 → 补集塌空 → 主断言 vacuous。
#[test]
fn command_surface_and_phase1_declared_set_are_both_readable() {
    let surface = command_surface();
    let declared = phase1_declared_commands();

    assert!(
        surface.len() >= 100,
        "命令注册表只读出 {} 条——真源读取逻辑已损坏，补集扫描不成立",
        surface.len()
    );
    assert!(
        declared.len() >= 15,
        "Phase 1 声明集只读出 {} 条——Gate 参考包读取逻辑已损坏",
        declared.len()
    );

    // 两个真源必须说同一套词汇，否则补集是「因为对不上而全部剩下」的假补集。
    for anchor in ["im_send_message", "im_urgent_post", "im_revoke"] {
        assert!(
            declared.contains(anchor),
            "Phase 1 声明集缺少锚点命令 `{anchor}`"
        );
        assert!(
            surface.contains(anchor),
            "命令注册表缺少锚点命令 `{anchor}`"
        );
    }

    let complement: Vec<&String> = surface.difference(&declared).collect();
    assert!(
        complement.len() >= 50,
        "注册表补集只剩 {} 条——声明集过宽（很可能读成了整张注册表），补集扫描会变成空转",
        complement.len()
    );
}

/// **主断言**：注册表里没有任何一条命令可归因于工具栏动作。
///
/// 扫描域是整张注册表（自动增长），不是手工候选名。补集只用于把违规分成两类报告：
/// 落在补集里 = 有人偷偷给工具栏动作加了命令；落在 Phase 1 声明集里 =
/// 参考包与本 Gate 语义冲突，须回 spec 层裁定（按 contracts/README.md:22，改实现不改样本）。
#[test]
fn no_command_surface_entry_is_attributable_to_a_toolbar_action() {
    let surface = command_surface();
    let declared = phase1_declared_commands();

    // 防空转：扫描域必须非空且规模合理（与前置测试同一守卫，独立成立）。
    assert!(
        surface.len() >= 100,
        "命令注册表只读出 {} 条，扫描域已损坏",
        surface.len()
    );

    // 探测器在**真实扫描域上**的正控：把反例并入一份副本，谓词必须逐条判出。
    // 这证明「真扫描域零命中」是「确实没有违规」，而不是「扫描器扫不出东西」。
    for exemplar in TOOLBAR_VIOLATION_EXEMPLARS {
        let mut probe = surface.clone();
        probe.insert(exemplar.to_string());
        let flagged: Vec<&String> = probe
            .iter()
            .filter(|name| is_toolbar_attributable(name))
            .collect();
        assert_eq!(
            flagged,
            vec![&exemplar.to_string()],
            "positive control: 注入 `{exemplar}` 后扫描器必须且只能判出它"
        );
    }

    let violations: Vec<String> = surface
        .iter()
        .filter(|name| is_toolbar_attributable(name))
        .map(|name| {
            let bucket = if declared.contains(name) {
                "Phase 1 声明集（参考包与 Gate 语义冲突，回 spec 裁定）"
            } else {
                "注册表补集（未声明命令：工具栏动作被升级成 Port 调用）"
            };
            format!("{name} [{bucket}]")
        })
        .collect();

    assert!(
        violations.is_empty(),
        "MV3-G05l：命令注册表出现可归因于工具栏动作的命令——纯本地交互不得升级成 Port 调用：\n  {}",
        violations.join("\n  ")
    );
}

/// 工具栏动作**标识符本身**既不是 outbound 命令，也不是 query，也不在公开别名表。
#[test]
fn toolbar_action_identifiers_are_not_dispatchable() {
    // 正控：分发判定函数本身有效（真命令必须被认出来）。
    assert!(
        helix_im::query::is_query("im_query_dialog_list"),
        "positive control: is_query 必须认出真 query"
    );
    assert!(
        helix_im::commands::is_outbound("im_urgent_post"),
        "positive control: is_outbound 必须认出真 outbound 命令"
    );
    assert!(
        helix_im::client_api::command_alias("im_send").is_some(),
        "positive control: command_alias 必须认出真别名"
    );

    for name in toolbar_identifiers() {
        assert!(
            !helix_im::commands::is_outbound(&name),
            "MV3-G05l：`{name}` 已被注册进 outbound registry"
        );
        assert!(
            !helix_im::query::is_query(&name),
            "MV3-G05l：`{name}` 已被注册进 query 表"
        );
        assert!(
            helix_im::client_api::command_alias(&name).is_none(),
            "MV3-G05l：`{name}` 出现在公开别名表里"
        );
    }
}

// ─── 措施 3：差分回放（「零 IM 状态面写入」的行为形断言）─────────────────────

/// 一次固定回放：前置入站消息 → 插入 `interleaved` → 收尾跑一条**真命令**。
///
/// 返回全过程 Effect 的逐条 `Debug` 全文（含 correlation / timer id / 请求体）。
/// 收尾真命令是「探针」：任何被 `interleaved` 悄悄改动的状态面（游标、correlation
/// 计数、pending 表、频道聚合）都会在探针产出的 Effect 里显形。
fn replay(interleaved: Vec<Tick>) -> Vec<String> {
    let mut harness = empty_scenario();
    let mut trace = Vec::new();

    let record = |harness: &Harness, label: &str, trace: &mut Vec<String>| {
        for effect in harness.sink.as_slice() {
            trace.push(format!("{label}|{effect:?}"));
        }
    };

    harness.step(inbound_event(1, 1, 0), NOW_MS);
    record(&harness, "prelude", &mut trace);

    for (index, tick) in interleaved.into_iter().enumerate() {
        harness.step(tick, NOW_MS + 1);
        record(&harness, &format!("interleaved#{index}"), &mut trace);
    }

    harness.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": common::cid(1),
                "type": "TEXT",
                "text": "probe",
                "props": {},
                "viewers": ["all"],
                "mentions": []
            }),
        ),
        NOW_MS + 2,
    );
    record(&harness, "probe", &mut trace);

    trace
}

/// 五个工具栏动作的 Tick 对 IM 状态面**完全透明**：插入前后，后续真命令产出的
/// Effect 全序列逐字节相同。
///
/// 这才是参考包 `negativeAssertions` 第三条（「绝不写任何 IM 状态面：unreadCount /
/// readBits / lastPost / mentionCount / draft / 消息集合全部零变化」）的行为形断言。
/// 对照组插入一条真 inbound 消息，必须让序列产生差异——否则差分装置没有鉴别力，
/// 上面的 `assert_eq!` 就是空转。
#[test]
fn toolbar_action_ticks_are_transparent_to_the_next_real_command() {
    let baseline = replay(Vec::new());
    assert!(
        baseline.len() >= 2,
        "差分基线只有 {} 条 Effect——回放装置没跑起来，差分结论不成立",
        baseline.len()
    );

    let toolbar_ticks: Vec<Tick> = toolbar_identifiers()
        .iter()
        .map(|name| owned_command(name, serde_json::json!({"channel_id": common::cid(1)})))
        .collect();
    let with_toolbar = replay(toolbar_ticks);
    assert_eq!(
        baseline, with_toolbar,
        "MV3-G05l：插入工具栏动作 Tick 后，后续真命令的 Effect 序列发生了变化——\
         说明工具栏动作写到了某个 IM 状态面"
    );

    // 对照组：插入一条真 inbound 消息，序列必须变化（证明差分装置有鉴别力）。
    let control = replay(vec![inbound_event(2, 1, 0)]);
    assert_ne!(
        baseline, control,
        "negative control: 插入真 inbound 消息都不改变 Effect 序列，差分装置无鉴别力，\
         上面的 assert_eq! 是空转"
    );
}

/// 工具栏动作 Tick 的四项零计数（参考包「专项断言」①②③④ 的字面复核）。
///
/// **本测试只是控制组，不是本 Gate 的覆盖来源**：dispatcher 对任何未认领的命令名都
/// no-op，任意随机字符串同样能通过。测试内显式跑一遍随机串来固化这个事实——
/// 谁把它当成工具栏专属证据，就会先看到这句注释和 `random_probe` 的等价性断言。
///
/// 真正的工具栏语义覆盖在：
/// - `no_command_surface_entry_is_attributable_to_a_toolbar_action`（注册表补集扫描）
/// - `toolbar_action_ticks_are_transparent_to_the_next_real_command`（状态面差分）
#[test]
fn unclaimed_command_ticks_are_inert_control_group() {
    // 空场景：构造后未喂 Tick，Effect 池必须为空。
    let mut harness = empty_scenario();
    assert!(
        SideEffectCounts::of(harness.sink.as_slice()).is_all_zero(),
        "空场景不得凭空产生副作用"
    );

    // 正控：真发送命令必须推动 storage 计数（证明本测试的计数器确实在读真 Effect）。
    harness.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": common::cid(1),
                "type": "TEXT",
                "text": "positive control",
                "props": {},
                "viewers": ["all"],
                "mentions": []
            }),
        ),
        NOW_MS,
    );
    let control = SideEffectCounts::of(harness.sink.as_slice());
    assert!(
        control.storage > 0,
        "positive control: 真发送必须产生 Storage 写入，got {control:?}"
    );

    // 工具栏动作标识符 + 一个随机串：两者行为必须**完全一致**（都零副作用）。
    // 这条等价性正是「本测试没有工具栏专属鉴别力」的证据，写死在测试里防止误读。
    let mut probes = toolbar_identifiers();
    probes.push("__mv3_g05l_random_probe_not_a_toolbar_action__".to_string());

    for name in probes {
        let mut harness = empty_scenario();
        harness.step(
            owned_command(&name, serde_json::json!({"channel_id": common::cid(1)})),
            NOW_MS,
        );
        let counts = SideEffectCounts::of(harness.sink.as_slice());
        assert!(
            counts.is_all_zero(),
            "MV3-G05l：`{name}` 产生了副作用 {counts:?}（要求 Command/Http/Storage/Event 四项恒为 0）"
        );
        // 连 Timer / Upload 这类非四项的 Effect 也不得出现（纯 UI 动作零编排）。
        assert!(
            harness.sink.as_slice().is_empty(),
            "MV3-G05l：`{name}` 产生了非空 Effect 批次"
        );
        assert_eq!(
            harness.module.pending_send_count(),
            0,
            "MV3-G05l：`{name}` 不得登记 PendingSend"
        );
    }
}
