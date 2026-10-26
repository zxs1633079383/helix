//! MessageV3 Gate `MV3-G02e`：草稿域（`im_save_draft` / `im_query_draft`）。
//!
//! 权威参考包：
//! `specs/003-messagev3-gate-inventory/gates/draft/mv3-g02e/`（visual 仓）。
//!
//! ## 合同要点（不得靠猜）
//!
//! - `effects-contract.json.allowed_effect_tags = ["Persist"]`：本 Gate **只能**产生
//!   `Effect::Persist`，不得 Http、不得 UploadFile、不得 ScheduleTimer。
//! - `outbound-contract.json.channels = []`：本 Gate **零领域事件**——草稿不占用 21 个
//!   `ImGateEvent` kind 中的任何一个。结果以**结构化 Command Result** 回灌，复用既有
//!   request/response 通道 `im:read:result{req_id, body}`（见 [`crate::read_relay`]，
//!   该通道被显式定义为「非投影」，Adapter 不把它映射成业务事件）。
//! - `outbound-sample.json.command_results[].rust = {"ok": true, "draft": DraftProjection}`。
//! - data-model §12：唯一约束 `(accountId, channelId)`；last-write-wins；
//!   **保存失败不得清空旧草稿**。
//! - INV-01：`accountId` 来自运行时身份（`ImConfig::auth_user_id`），
//!   Angular 不得提交——payload 里出现 `account_id` 一律 fail-closed。
//!
//! ## 失败即保旧草稿
//!
//! 保存链是 `Persist -> PersistOk -> Command Result`：结果事件绑定在
//! [`crate::state::CorrelationContext::MessageV3Commit`] 上，只有真实 `PersistOk` 才释放。
//! `PersistErr` 时 SQL 事务未提交 → 旧行原样保留，且零 Command Result、零事件。

use helix_core::effect::{Effect, Row, ScopedGetSpec, SqlValue, StorageOp, UpsertSpec};
use helix_core::EffectSink;
use serde_json::{Map, Value};

use crate::error::ImError;
use crate::event::MessageV3Event;
use crate::module::ImModule;
use crate::state::CorrelationContext;

/// 草稿唯一持久化表。DDL 已在 [`crate::schema`] 定义，本模块不重复定义 schema。
pub const DRAFT_TABLE: &str = "message_draft";

/// 结构化 Command Result 的回灌通道（非业务事件；Adapter 无对应 `ImGateEvent` kind）。
pub const DRAFT_RESULT_EVENT: &str = "im:read:result";

const SAVE_COMMAND: &str = "im_save_draft";
const QUERY_COMMAND: &str = "im_query_draft";

/// `im_save_draft` 公开边界允许的**全部**顶层键；其余一律 fail-closed。
const SAVE_KEYS: &[&str] = &["channel_id", "text", "props", "updated_at", "req_id"];
/// `im_query_draft` 公开边界允许的**全部**顶层键；其余一律 fail-closed。
const QUERY_KEYS: &[&str] = &["channel_id", "req_id"];

// ── 投影 ────────────────────────────────────────────────────────────────────

/// data-model §12 的绝对草稿态。Angular 直接 Upsert，无需二次业务计算。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftProjection {
    pub account_id: String,
    pub channel_id: String,
    pub text: String,
    pub props: Map<String, Value>,
    pub updated_at: i64,
}

impl DraftProjection {
    /// 输出 Angular `DraftProjection` 形状（camelCase 绝对态）。
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "accountId": self.account_id,
            "channelId": self.channel_id,
            "text": self.text,
            "props": Value::Object(self.props.clone()),
            "updatedAt": self.updated_at,
        })
    }

    /// 输出持久化行；列名与 `message_draft` DDL 逐字对齐。
    pub fn to_row(&self) -> Row {
        vec![
            (
                "account_id".to_string(),
                SqlValue::Text(self.account_id.clone()),
            ),
            (
                "channel_id".to_string(),
                SqlValue::Text(self.channel_id.clone()),
            ),
            ("text".to_string(), SqlValue::Text(self.text.clone())),
            (
                "props".to_string(),
                SqlValue::Text(
                    serde_json::to_string(&Value::Object(self.props.clone()))
                        .unwrap_or_else(|_| "{}".to_string()),
                ),
            ),
            ("updated_at".to_string(), SqlValue::Integer(self.updated_at)),
        ]
    }

    /// 从 durable row 还原绝对草稿态。
    ///
    /// `account_id` / `channel_id` 是复合主键，缺任一列 → `None`（不猜、不补默认账号）。
    /// `props` 列是本地自写的 JSON 文本，坏值降级为 `{}`（不 panic，也不吞掉整行）。
    pub fn from_row(row: &Row) -> Option<Self> {
        let account_id = text_column(row, "account_id")?.to_string();
        let channel_id = text_column(row, "channel_id")?.to_string();
        if account_id.is_empty() || channel_id.is_empty() {
            return None;
        }
        Some(Self {
            account_id,
            channel_id,
            text: text_column(row, "text").unwrap_or_default().to_string(),
            props: text_column(row, "props")
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default(),
            updated_at: integer_column(row, "updated_at").unwrap_or_default(),
        })
    }

    /// `INSERT OR REPLACE`（`conflict_key = None`）在 `PRIMARY KEY (account_id, channel_id)`
    /// 上天然收敛为 last-write-wins，且写复杂度 O(1)（无 read-modify-write）。
    pub fn upsert_op(&self) -> StorageOp {
        StorageOp::BatchUpsert(UpsertSpec {
            version_column: None,
            update_guard: None,
            table: DRAFT_TABLE,
            rows: vec![self.to_row()],
            conflict_key: None,
            exclude_from_update: Vec::new(),
        })
    }
}

// ── Command 解析（forbid_unknown / 越权 fail-closed）─────────────────────────

/// `im_save_draft` 的已校验入参。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDraftCommand {
    pub draft: DraftProjection,
    pub req_id: Option<String>,
}

impl SaveDraftCommand {
    /// 解析并校验保存意图。
    ///
    /// `account_id` 只来自运行时身份；`updated_at` 缺省用 Host Clock `now_ms`（data-model §12）。
    pub fn parse(payload: &[u8], account_id: &str, now_ms: u64) -> Result<Self, ImError> {
        let obj = as_object(payload, SAVE_COMMAND)?;
        reject_unknown(&obj, SAVE_KEYS, SAVE_COMMAND)?;
        let account_id = require_account(account_id, SAVE_COMMAND)?;
        let channel_id = require_channel_id(&obj, SAVE_COMMAND)?;
        let text = match obj.get("text") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(text)) => text.clone(),
            Some(_) => {
                return Err(ImError::Parse(format!("{SAVE_COMMAND}: text 必须是字符串")));
            }
        };
        let props = match obj.get("props") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(props)) => props.clone(),
            Some(_) => {
                return Err(ImError::Parse(format!("{SAVE_COMMAND}: props 必须是对象")));
            }
        };
        let updated_at = match obj.get("updated_at") {
            None | Some(Value::Null) => now_ms as i64,
            Some(value) => value.as_i64().filter(|value| *value > 0).ok_or_else(|| {
                ImError::Parse(format!("{SAVE_COMMAND}: updated_at 必须是正整数"))
            })?,
        };
        Ok(Self {
            draft: DraftProjection {
                account_id,
                channel_id,
                text,
                props,
                updated_at,
            },
            req_id: optional_req_id(&obj),
        })
    }

    /// 保存成功后的结构化 Command Result（对齐 `outbound-sample.json`）。
    pub fn result_event(&self) -> Result<MessageV3Event, ImError> {
        result_event(self.req_id.as_deref(), Some(&self.draft))
    }
}

/// `im_query_draft` 的已校验入参。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDraftCommand {
    pub account_id: String,
    pub channel_id: String,
    pub req_id: Option<String>,
}

impl QueryDraftCommand {
    /// 解析并校验查询意图。
    pub fn parse(payload: &[u8], account_id: &str) -> Result<Self, ImError> {
        let obj = as_object(payload, QUERY_COMMAND)?;
        reject_unknown(&obj, QUERY_KEYS, QUERY_COMMAND)?;
        Ok(Self {
            account_id: require_account(account_id, QUERY_COMMAND)?,
            channel_id: require_channel_id(&obj, QUERY_COMMAND)?,
            req_id: optional_req_id(&obj),
        })
    }

    /// 复合主键 O(1) 单行读；`scope_col = account_id` 保证**绝不跨账号**读到别人的草稿。
    pub fn get_op(&self) -> StorageOp {
        StorageOp::ScopedGet(ScopedGetSpec {
            table: DRAFT_TABLE,
            scope_col: "account_id",
            scope_val: SqlValue::Text(self.account_id.clone()),
            key_col: "channel_id",
            key_val: SqlValue::Text(self.channel_id.clone()),
        })
    }

    /// 把 read-back rows 投影成结构化 Command Result。
    ///
    /// 账号隔离在此二次收敛：即使 driver 回了别的账号/频道的行，也一律当作 miss
    /// （`draft: null`），不泄漏跨账号内容。
    pub fn result_event(&self, rows: &[Row]) -> Result<MessageV3Event, ImError> {
        let draft = rows
            .iter()
            .find_map(DraftProjection::from_row)
            .filter(|draft| {
                draft.account_id == self.account_id && draft.channel_id == self.channel_id
            });
        result_event(self.req_id.as_deref(), draft.as_ref())
    }
}

/// 构造 `im:read:result{req_id, body:{ok, draft}}` —— 零领域事件的结构化结果通道。
fn result_event(
    req_id: Option<&str>,
    draft: Option<&DraftProjection>,
) -> Result<MessageV3Event, ImError> {
    MessageV3Event::new(
        DRAFT_RESULT_EVENT,
        serde_json::json!({
            "req_id": req_id.unwrap_or_default(),
            "body": {
                "ok": true,
                "draft": draft.map(DraftProjection::to_json).unwrap_or(Value::Null),
            },
        }),
    )
}

// ── 模块入口 ────────────────────────────────────────────────────────────────

/// 本 Gate 在 `Tick::Command` 上认领的**全部**公开命令名。
///
/// 与 [`crate::client_api::command`] 别名表同名（草稿域无重命名），是
/// `Module::accepts` 与 `Module::handle` 共用的**唯一**判定源——两处若各写一份
/// 字符串常量，就会重演「accepts 漏放行 → 整族命令静默丢弃」的旧事故。
pub const DRAFT_COMMANDS: &[&str] = &[SAVE_COMMAND, QUERY_COMMAND];

/// 该命令名是否归草稿域处理。
pub fn is_draft_command(name: &str) -> bool {
    DRAFT_COMMANDS.contains(&name)
}

/// `Tick::Command` 分发入口：把 [`DRAFT_COMMANDS`] 路由到对应 handler。
///
/// 未认领的命令名一律 `Err`（fail-closed）——本函数只应在
/// [`is_draft_command`] 为真时被调用，走到 `_` 分支即调用方路由写错了。
pub fn handle_command(
    module: &mut ImModule,
    name: &str,
    payload: &[u8],
    now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    match name {
        SAVE_COMMAND => handle_save_draft(module, payload, now_ms, out),
        QUERY_COMMAND => {
            handle_query_draft(module, payload, out)?;
            Ok(())
        }
        other => Err(ImError::Parse(format!("draft: 未认领的命令 {other}"))),
    }
}

/// `im_save_draft`：Persist 绝对草稿态，PersistOk 后释放唯一 Command Result。
///
/// 产出恰好一条 `Effect::Persist`（合同 `allowed_effect_tags = ["Persist"]`），
/// 零 Http、零领域事件。`PersistErr` → 零结果、旧草稿保留。
pub fn handle_save_draft(
    module: &mut ImModule,
    payload: &[u8],
    now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let command = SaveDraftCommand::parse(payload, module.config.auth_user_id.as_str(), now_ms)?;
    let terminal = command.result_event()?.into_bytes();
    let corr = module.alloc_corr_internal();
    module.state.corr_map.insert(
        corr,
        CorrelationContext::MessageV3Commit {
            terminal_events: vec![terminal],
        },
    );
    out.push(Effect::Persist {
        corr,
        ops: vec![command.draft.upsert_op()],
    });
    Ok(())
}

/// `im_query_draft`：按 `(accountId, channelId)` 单行读，零领域事件。
///
/// 返回本次读的 [`QueryDraftCommand`]，matching PortReply continuation 用
/// [`QueryDraftCommand::result_event`] 把 rows 投影成 Command Result。
pub fn handle_query_draft(
    module: &mut ImModule,
    payload: &[u8],
    out: &mut EffectSink,
) -> Result<QueryDraftCommand, ImError> {
    let command = QueryDraftCommand::parse(payload, module.config.auth_user_id.as_str())?;
    let corr = module.alloc_corr_internal();
    module.state.corr_map.insert(
        corr,
        CorrelationContext::MessageV3DraftReadback {
            command: Box::new(command.clone()),
        },
    );
    out.push(Effect::Persist {
        corr,
        ops: vec![command.get_op()],
    });
    Ok(command)
}

// ── 局部 helper（边界零信任：缺/类型错 → Err，不 panic）──────────────────────

fn as_object(payload: &[u8], cmd: &str) -> Result<Map<String, Value>, ImError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("{cmd} payload: {error}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| ImError::Parse(format!("{cmd} payload 必须是对象")))
}

fn reject_unknown(obj: &Map<String, Value>, allowed: &[&str], cmd: &str) -> Result<(), ImError> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ImError::Parse(format!("{cmd}: 未知字段 {key}")));
        }
    }
    Ok(())
}

fn require_account(account_id: &str, cmd: &str) -> Result<String, ImError> {
    if account_id.is_empty() {
        return Err(ImError::Parse(format!("{cmd}: 缺运行时账号身份")));
    }
    Ok(account_id.to_string())
}

fn require_channel_id(obj: &Map<String, Value>, cmd: &str) -> Result<String, ImError> {
    obj.get("channel_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 channel_id")))
}

fn optional_req_id(obj: &Map<String, Value>) -> Option<String> {
    obj.get("req_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn text_column<'a>(row: &'a Row, column: &str) -> Option<&'a str> {
    row.iter().find_map(|(name, value)| match value {
        SqlValue::Text(value) if name == column => Some(value.as_str()),
        _ => None,
    })
}

fn integer_column(row: &Row, column: &str) -> Option<i64> {
    row.iter().find_map(|(name, value)| match value {
        SqlValue::Integer(value) if name == column => Some(*value),
        _ => None,
    })
}
