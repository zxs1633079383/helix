//! bot-agent 域 outbound 命令（P5b，parallelGroup=G2）。
//!
//! 与 P5a `user_misc.rs` 真零交集：独立目录模块 + 各命令 distributed_slice 注册，不碰
//! registry.rs（除 P5b 为接 `X-Auth-Kind: bot` 给 `OutboundCommand` 加**默认**方法 `auth_kind()`
//! 一处向后兼容扩展）/channel*/posts_read.rs/user_misc.rs；`outbound/mod.rs` 仅 append 一行。
//!
//! ## 为什么是目录模块（many small files，code-structure-ddd §1/§5）
//! 本域 36 命令（6 子族）一文件必破 300 行硬顶——按子族裂分：gateway / bot_manage / config /
//! calls / agents / webhook 各一文件，本 `mod.rs` 持共享 `bot_cmd!`/`bot_call_cmd!` 宏 + helper，
//! 只 re-export（submodule 在宏**之后**声明，方可用宏，macro_rules 文本作用域）。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字 json tag，不自造 fixture，C5）
//! - posts/* = `api/cses/posts`（api.go:169）：createBot/botCallback（bot.go:15/18）。
//! - bot-manage/* = `api/cses/bot-manage`（api.go:184，bot_manage.go:14-22）。
//! - bot-agent/* = `api/cses/bot-agent`（api.go:185，bot_agent.go）。
//! - agents/* = `api/cses/agents`（api.go:170，agent.go:14-18）。
//! - webhook/* = `api/cses` 根（webhook_dispatcher.go:13-15）。
//!
//! ## 鉴权意图（C6 / HX-C001 铁律）
//! bot-token 调用命令（#22-27 info/messages/teams）`auth_kind()` override 为 `Bot` →
//! `Effect::Http` 带 `X-Auth-Kind: bot`，**真凭据由 driver 横切层注入**，core 绝不出现 bot 凭据
//! 字面量 / 鉴权头名（C6 闸门：本 crate grep token 串必空）。其余命令走默认 `Session`。
//!
//! ## method / path-param / query-param（build 层只保证 endpoint+body，method 属 driver 接缝 P6）
//! 现网本域含 GET(#3/6/11/13/14/18/19/22/25/26/27/36)/DELETE(#15/21)；registry 恒发 POST，
//! method 差异由 driver 接缝（P6）按命令名映射校正（与 P5a teams/member/quit DELETE 同款裁决）。
//! path param（`{botUserId}`/`{channelId}`/`{teamId}`/`{userId}`）+ query（`?botUserId=`）在
//! build 层插值进 path 字符串（endpoint 对齐现网）。path 段做最小转义校验（拒空/拒含 `/` 防越权）。
//!
//! ## 响应态特例（outbound 只 build 请求体，不裁决响应）
//! - #2/#32 内存构造 post **不推 cursor**（botCallback/agentCallback 内存 publish，不写 channel_event，
//!   gap-bot-agent.md:14/44）——属 WS 推送/落库态，与 outbound build 无关，文档断言锚边界。
//! - #22/#35-37 响应裸 `map`（非 `dto.CommonRes`）——响应态，outbound 不解析响应，文档断言锚边界。
//! - timeline/cost（#33/34）+ cross-repo demo（#38-40）**不接管**（D7 废弃 / 非业务），不注册。

use serde_json::Value;

use crate::error::ImError;

/// 校验并取一个 path 段（非空、不含 `/`，防路径注入/越权）。供 `{botUserId}`/`{channelId}` 等插值。
pub(super) fn path_seg<'a>(args: &'a Value, key: &str, cmd: &str) -> Result<&'a str, ImError> {
    let v = args
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && !s.contains('/'))
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/坏 path 段 '{key}'（非空且不含 '/'）")))?;
    Ok(v)
}

/// 内部：注册三件套（struct + impl OutboundCommand + distributed_slice）。
/// `$auth` = `Session`|`Bot`；`$path_ov` = 0（静态）|动态 path 闭包。
macro_rules! __outbound_impl {
    // 静态 path 命令（build 产 (&'static str, body)）。
    (static $cmd:ident, $reg:ident, $name:literal, $auth:ident, $build:expr) => {
        struct $cmd;
        impl $crate::outbound::registry::OutboundCommand for $cmd {
            fn name(&self) -> &'static str { $name }
            fn build(&self, a: &::serde_json::Value)
                -> ::std::result::Result<(&'static str, ::serde_json::Value), $crate::error::ImError> {
                let f: fn(&::serde_json::Value, &'static str)
                    -> ::std::result::Result<(&'static str, ::serde_json::Value), $crate::error::ImError> = $build;
                f(a, $name)
            }
            fn auth_kind(&self) -> ::helix_core::AuthKind { ::helix_core::AuthKind::$auth }
        }
        __outbound_impl!(@register $cmd, $reg, $name);
    };
    // 动态 path 命令（build 产 (base_path, body)；path_override 产完整动态 path）。
    (dyn $cmd:ident, $reg:ident, $name:literal, $base:literal, $auth:ident, $path:expr, $body:expr) => {
        struct $cmd;
        impl $crate::outbound::registry::OutboundCommand for $cmd {
            fn name(&self) -> &'static str { $name }
            fn build(&self, a: &::serde_json::Value)
                -> ::std::result::Result<(&'static str, ::serde_json::Value), $crate::error::ImError> {
                let f: fn(&::serde_json::Value, &'static str)
                    -> ::std::result::Result<::serde_json::Value, $crate::error::ImError> = $body;
                Ok(($base, f(a, $name)?))
            }
            fn path_override(&self, a: &::serde_json::Value)
                -> ::std::option::Option<::std::result::Result<::std::string::String, $crate::error::ImError>> {
                let f: fn(&::serde_json::Value, &'static str)
                    -> ::std::result::Result<::std::string::String, $crate::error::ImError> = $path;
                ::std::option::Option::Some(f(a, $name))
            }
            fn auth_kind(&self) -> ::helix_core::AuthKind { ::helix_core::AuthKind::$auth }
        }
        __outbound_impl!(@register $cmd, $reg, $name);
    };
    (@register $cmd:ident, $reg:ident, $name:literal) => {
        #[::linkme::distributed_slice($crate::outbound::registry::OUTBOUND_COMMANDS)]
        static $reg: $crate::outbound::registry::OutboundRegistration =
            $crate::outbound::registry::OutboundRegistration { name: $name, command: &$cmd };
    };
}

/// session 静态 path 命令。`$build`: `fn(&Value,&str)->Result<(&'static str,Value)>`。
macro_rules! bot_cmd {
    ($cmd:ident, $reg:ident, $name:literal, $build:expr) => {
        __outbound_impl!(static $cmd, $reg, $name, Session, $build);
    };
}
/// bot-token（auth=Bot）静态 path 命令（C6：core 只标 `X-Auth-Kind: bot` 意图，token 留 driver）。
macro_rules! bot_call_cmd {
    ($cmd:ident, $reg:ident, $name:literal, $build:expr) => {
        __outbound_impl!(static $cmd, $reg, $name, Bot, $build);
    };
}
/// session 动态 path 命令。`$base`=静态 fallback；`$path`: args→Result<String>；`$body`: args→Result<Value>。
macro_rules! bot_dyn_cmd {
    ($cmd:ident, $reg:ident, $name:literal, $base:literal, $path:expr, $body:expr) => {
        __outbound_impl!(dyn $cmd, $reg, $name, $base, Session, $path, $body);
    };
}
/// bot-token 动态 path 命令（auth=Bot + path_override）。
macro_rules! bot_call_dyn_cmd {
    ($cmd:ident, $reg:ident, $name:literal, $base:literal, $path:expr, $body:expr) => {
        __outbound_impl!(dyn $cmd, $reg, $name, $base, Bot, $path, $body);
    };
}

// submodule 在宏**之后**声明（macro_rules 文本作用域：声明前不可见）。各子族独立文件 ≤300 行。
pub(super) mod agents;
pub(super) mod bot_manage;
pub(super) mod calls;
pub(super) mod config;
pub(super) mod gateway;
pub(super) mod webhook;
