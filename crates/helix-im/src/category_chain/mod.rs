//! 分类接龙独立领域接缝；authority 来自服务端，所有成功结果都经过存储读回。
pub(crate) mod facts;
pub(crate) mod post;
pub(crate) mod post_events;
pub(crate) mod runtime;
pub(crate) mod wire;

use helix_core::{Correlation, TimerId};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

pub(crate) const EVENT: &str = "im:category_chain:projection";
pub(crate) const STATUS_EVENT: &str = "im:category_chain:operation";

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub command: String,
    pub channel: String,
    pub chain: Option<String>,
    pub viewer: String,
    pub req_id: Option<String>,
    pub mutation: Option<String>,
    pub query_scope: String,
    pub temporary_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub scope: String,
    pub revision: String,
    pub value: Value,
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Work {
    pub request: Request,
    pub documents: Vec<Document>,
    pub visibility_scope: String,
    pub post_guard: Option<String>,
    /// Time/access dependent permissions belong to this response, not the versioned participant fact.
    pub permissions: Option<Value>,
    pub index: usize,
    pub data: Value,
    pub event_seq: Option<crate::state::Seq>,
    pub ws: bool,
    pub reconcile: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    Prepare { request: Request, payload: Vec<u8> },
    Http { request: Request, timer: TimerId },
    Visibility(Work),
    Head(Work),
    MessageHead(Work),
    MessageReadback(Work),
    Persist(Work),
    Readback(Work),
}

#[derive(Debug, Default)]
pub(crate) struct State {
    pub queue: VecDeque<Work>,
    pub busy: bool,
    pub timers: HashMap<TimerId, Correlation>,
}

/// 精确命令集合；未知分类命令不能静默落入文字接龙。
pub(crate) fn is_command(name: &str) -> bool {
    wire::COMMANDS.contains(&name)
}

/// 区分需要读回 waiter 的查询，不把 accepted 当查询结果。
pub(crate) fn is_read(name: &str) -> bool {
    matches!(
        name,
        "category_chain_capabilities"
            | "category_chain_get"
            | "category_chain_get_mine"
            | "category_chain_entries"
            | "category_chain_reconcile"
    )
}

/// 读取服务端 canonical 字符串身份；缺失时保持缺失。
pub(crate) fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
