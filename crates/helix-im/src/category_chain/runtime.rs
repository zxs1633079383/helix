//! 单队列串行 authority 检查/写入；存储仍由 Effect 执行，core 无 I/O 或时间来源。
use super::{facts, string, wire, Document, Pending, Request, Work, EVENT, STATUS_EVENT};
use crate::state::{CorrelationContext, ImState};
use crate::{ImError, ImModule};
use helix_core::tick::{PortError, PortOutcome};
use helix_core::{Correlation, Effect, EffectSink, TimerId};
use serde_json::{json, Value};

const HISTORICAL_RECONCILE_GUARD: &str = "__category_chain_historical_reconcile__";

impl ImModule {
    /// 校验最小意图后注册 HTTP 和超时；失败不残留 correlation。
    pub(crate) fn handle_category_command(
        &mut self,
        command: &str,
        payload: &[u8],
        _now: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if command != "category_chain_capabilities"
            && (self.config.auth_user_id.is_empty() || self.config.company_id.is_empty())
        {
            return Err(invalid("runtime identity required"));
        }
        if payload.len() > 256 * 1024 {
            return Err(invalid("request too large"));
        }
        let mut args: Value =
            serde_json::from_slice(payload).map_err(|e| ImError::Parse(e.to_string()))?;
        if !args.is_object() {
            return Err(invalid("request object"));
        }
        if command == "category_chain_publish" {
            if args.get("temporary_id").is_some() || args.get("temporaryId").is_some() {
                return Err(invalid("temporaryId belongs to Helix"));
            }
            if self.config.company_id.is_empty() || self.config.auth_user_id.is_empty() {
                return Err(invalid("runtime identity required"));
            }
            use sha2::{Digest, Sha256};
            let key = json!([
                self.config.company_id,
                self.config.auth_user_id,
                args.get("channel_id"),
                args.get("client_mutation_id")
            ]);
            let hash = Sha256::digest(key.to_string().as_bytes());
            let id = format!("cc{:x}", hash);
            args["temporary_id"] = json!(&id[..26]);
        }
        wire::wire_body(command, &args)?;
        let request = Request {
            command: command.to_owned(),
            channel: string(&args, "channel_id").unwrap_or_default(),
            chain: string(&args, "chain_id"),
            viewer: self.config.auth_user_id.to_owned(),
            req_id: string(&args, "req_id"),
            mutation: string(&args, "client_mutation_id"),
            query_scope: json!([
                args.get("category_id"),
                args.get("cursor"),
                args.get("limit")
            ])
            .to_string(),
            temporary_id: string(&args, "temporary_id"),
        };
        if super::is_read(command) && request.req_id.is_none() {
            return Err(invalid("req_id"));
        }
        let body = serde_json::to_vec(&args).map_err(|e| invalid(&e.to_string()))?;
        if command == "category_chain_publish" {
            let scope = json!([
                self.config.company_id,
                request.viewer,
                request.channel,
                request.mutation,
                "publish-identity"
            ])
            .to_string();
            let ops = facts::persist(&scope, "0", &json!({"temporaryId":args["temporary_id"]}))?;
            let corr = self.alloc_corr_internal();
            self.state.corr_map.insert(
                corr,
                CorrelationContext::CategoryChain {
                    pending: Box::new(Pending::Prepare {
                        request,
                        payload: body,
                    }),
                },
            );
            out.push(Effect::PersistAtomic { corr, ops });
            return Ok(());
        }
        self.start_category_http(request, &body, out)
    }

    /// 持久身份已确定后发HTTP；所有请求共用host鉴权与连接头。
    fn start_category_http(
        &mut self,
        request: Request,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let effects = crate::commands::handle_outbound(
            &request.command,
            payload,
            &self.config.api_base_url,
            &self.config.default_api_base_url,
            self.state.connection_id.as_deref(),
            corr,
        )?;
        let timer = self.alloc_timer();
        self.state.category_chain.timers.insert(timer, corr);
        self.state.corr_map.insert(
            corr,
            CorrelationContext::CategoryChain {
                pending: Box::new(Pending::Http { request, timer }),
            },
        );
        for effect in effects {
            out.push(effect);
        }
        out.push(Effect::ScheduleTimer {
            id: timer,
            after_ms: 30_000,
        });
        Ok(())
    }

    /// HTTP 超时移除 correlation，迟到回报无效；原 mutation 必须经 reconcile 查询。
    pub(crate) fn handle_category_timeout(
        &mut self,
        timer: TimerId,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let Some(corr) = self.state.category_chain.timers.remove(&timer) else {
            return Ok(false);
        };
        if let Some(CorrelationContext::CategoryChain { pending }) =
            self.state.corr_map.remove(&corr)
        {
            if let Pending::Http { request, .. } = *pending {
                fail(&request, "RECONCILING", "TRANSPORT_TIMEOUT", out)?;
            }
        }
        Ok(true)
    }

    /// stop/account reset 清理队列和timer；取消消费不伪造服务端回滚。
    pub(crate) fn cancel_category(&mut self, out: &mut EffectSink) {
        for (timer, _) in self.state.category_chain.timers.drain() {
            out.push(Effect::CancelTimer { id: timer });
        }
        for work in self.state.category_chain.queue.drain(..) {
            let _ = fail(&work.request, "RECONCILING", "CATEGORY_CANCELLED", out);
        }
        self.state.category_chain.busy = false;
        let contexts = std::mem::take(&mut self.state.corr_map);
        for (corr, context) in contexts {
            match context {
                CorrelationContext::CategoryChain { pending } => {
                    let request = match *pending {
                        Pending::Prepare { request, .. } | Pending::Http { request, .. } => request,
                        Pending::Visibility(work)
                        | Pending::Head(work)
                        | Pending::MessageHead(work)
                        | Pending::MessageReadback(work)
                        | Pending::Persist(work)
                        | Pending::Readback(work) => work.request,
                    };
                    let _ = fail(&request, "RECONCILING", "CATEGORY_CANCELLED", out);
                }
                CorrelationContext::CategoryPostReadback { .. } => {}
                other => {
                    self.state.corr_map.insert(corr, other);
                }
            }
        }
    }

    /// 公开 PortReply 的分类 continuation，每个阶段只消费一次 matching correlation。
    pub(crate) fn handle_category_reply(
        &mut self,
        pending: Pending,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Pending::Prepare { request, payload } = pending {
            return if matches!(outcome, PortOutcome::Ok(_)) {
                self.start_category_http(request, &payload, out)
            } else {
                fail(&request, "RECONCILING", "PERSIST_IDENTITY_FAILED", out)
            };
        }
        if let Pending::Http { request, timer } = pending {
            return self.handle_category_http_result(request, timer, outcome, out);
        }
        let request = match &pending {
            Pending::Visibility(work)
            | Pending::Head(work)
            | Pending::MessageHead(work)
            | Pending::MessageReadback(work)
            | Pending::Persist(work)
            | Pending::Readback(work) => work.request.clone(),
            Pending::Http { .. } | Pending::Prepare { .. } => unreachable!(),
        };
        let result = self.continue_category(pending, outcome, out);
        if let Err(error) = result {
            tracing::warn!(command = %request.command, error = %error, "category durable continuation rejected");
            self.finish_category(out)?;
            if request.mutation.is_none() && request.req_id.is_none() {
                return Err(error);
            }
            fail(&request, "RECONCILING", &error.to_string(), out)?;
        }
        Ok(())
    }

    /// HTTP authority校验独立收口，错误仅复制有界关联标识用于终态。
    fn handle_category_http_result(
        &mut self,
        request: Request,
        timer: TimerId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.category_chain.timers.remove(&timer);
        out.push(Effect::CancelTimer { id: timer });
        let data = match outcome {
            PortOutcome::Ok(bytes) => match wire::decode_response(bytes.0.as_ref()) {
                Ok(data) => data,
                Err(error) => {
                    let message = error.to_string();
                    let status = if message.contains("CATEGORY_CHAIN_REJECTED:") {
                        "REJECTED"
                    } else {
                        "RECONCILING"
                    };
                    fail(&request, status, &message, out)?;
                    return Ok(());
                }
            },
            PortOutcome::Err(error) => {
                let status = if matches!(error, PortError::Http(400..=499)) {
                    "REJECTED"
                } else {
                    "RECONCILING"
                };
                fail(&request, status, "TRANSPORT_FAILED", out)?;
                return Ok(());
            }
        };
        if request.command == "category_chain_capabilities" {
            if data
                .pointer("/categoryChain/schemaVersion")
                .and_then(Value::as_u64)
                != Some(1)
                || data
                    .pointer("/categoryChain/supported")
                    .and_then(Value::as_bool)
                    .is_none()
            {
                return fail(&request, "REJECTED", "CATEGORY_CHAIN_UNAVAILABLE", out);
            }
            if let Some(req_id) = &request.req_id {
                out.push(crate::query::read_relay::emit_read_body(
                    req_id,
                    json!({"status":"SUCCESS","data":data}),
                ));
            }
            return Ok(());
        }
        if let Err(error) =
            wire::validate_authority(&data, &request.channel, request.chain.as_deref(), true)
        {
            fail(&request, "RECONCILING", &error.to_string(), out)?;
            return Ok(());
        }
        if !super::is_read(&request.command) {
            let expected = request
                .command
                .strip_prefix("category_chain_")
                .unwrap_or("")
                .replace('_', "-");
            if data.get("commandKind").and_then(Value::as_str) != Some(expected.as_str()) {
                return fail(&request, "RECONCILING", "COMMAND_SCOPE_MISMATCH", out);
            }
        }
        if data.pointer("/card/revision").is_some()
            && data.pointer("/post/props/categoryChain").is_some()
            && data.get("card") != data.pointer("/post/props/categoryChain")
        {
            return fail(&request, "RECONCILING", "CARD_POST_MISMATCH", out);
        }
        if request.temporary_id.as_ref().is_some_and(|id| {
            data.pointer("/post/temporaryId").and_then(Value::as_str) != Some(id.as_str())
        }) {
            return fail(&request, "RECONCILING", "PUBLISH_IDENTITY_MISMATCH", out);
        }
        let mine = if request.command == "category_chain_get_mine" {
            Some(&data)
        } else {
            data.get("myParticipation")
                .or_else(|| data.pointer("/result/myParticipation"))
        };
        if mine
            .and_then(|m| m.get("entries"))
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry.get("userId").and_then(Value::as_str) != Some(request.viewer.as_str())
                })
            })
        {
            return fail(&request, "RECONCILING", "VIEWER_SCOPE_MISMATCH", out);
        }
        if let Some(id) = &request.mutation {
            if data.get("clientMutationId").and_then(Value::as_str) != Some(id.as_str()) {
                return fail(&request, "RECONCILING", "MUTATION_SCOPE_MISMATCH", out);
            }
        }
        let error_request = request.clone(); // Bounded correlation only, never copy authority.
        let work = match make_work(request, data, None, false) {
            Ok(work) => work,
            Err(error) => return fail(&error_request, "RECONCILING", &error.to_string(), out),
        };
        self.with_state_and_corr_allocator(|state, alloc| enqueue(state, alloc, work, out))?;
        return Ok(());
    }

    /// 存储阶段按 head→atomic→readback 顺序推进，绝不把 in-memory authority 当读回。
    fn continue_category(
        &mut self,
        pending: Pending,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(bytes) = outcome else {
            return Err(invalid("PERSIST_FAILED"));
        };
        match pending {
            Pending::Visibility(work) => {
                if facts::head_revision(bytes.0.as_ref())?.is_some()
                    && !work.documents.iter().any(|d| {
                        d.value.pointer("/card/status").and_then(Value::as_str) == Some("RETRACTED")
                    })
                {
                    return Err(invalid("CATEGORY_CHAIN_RETRACTED"));
                }
                let corr = self.alloc_corr_internal();
                let op = facts::head_op(&work.documents[0].scope);
                self.state.corr_map.insert(
                    corr,
                    CorrelationContext::CategoryChain {
                        pending: Box::new(Pending::Head(work)),
                    },
                );
                out.push(Effect::Persist {
                    corr,
                    ops: vec![op],
                });
            }
            Pending::Head(mut work) => {
                let mut historical = false;
                if let Some(previous) = facts::head_revision(bytes.0.as_ref())? {
                    let revision = work.documents[work.index].revision.clone();
                    match facts::revision_cmp(&previous, &revision)? {
                        std::cmp::Ordering::Greater => {
                            if work.reconcile {
                                // Reconcile may persist its private receipt while public facts stay newer.
                                work.documents[work.index].write = false;
                                historical = true;
                            } else {
                                return Err(invalid("STALE_AUTHORITY_RECONCILE"));
                            }
                        }
                        std::cmp::Ordering::Equal => work.documents[work.index].write = false,
                        std::cmp::Ordering::Less => {}
                    }
                }
                if historical {
                    work.post_guard = Some(HISTORICAL_RECONCILE_GUARD.to_owned());
                }
                work.index += 1;
                if work.index < work.documents.len() {
                    let corr = self.alloc_corr_internal();
                    let op = facts::head_op(&work.documents[work.index].scope);
                    self.state.corr_map.insert(
                        corr,
                        CorrelationContext::CategoryChain {
                            pending: Box::new(Pending::Head(work)),
                        },
                    );
                    out.push(Effect::Persist {
                        corr,
                        ops: vec![op],
                    });
                } else {
                    if historical_reconcile(&work) {
                        // A confirmed reconciliation is a private receipt; its old Post is never rewritten.
                        self.persist_category(work, out)?;
                    } else if let Some((_, temporary_id, _)) = message_snapshot(&work) {
                        let id = temporary_id.to_owned();
                        let corr = self.alloc_corr_internal();
                        self.state.corr_map.insert(
                            corr,
                            CorrelationContext::CategoryChain {
                                pending: Box::new(Pending::MessageHead(work)),
                            },
                        );
                        out.push(Effect::Persist {
                            corr,
                            ops: vec![helix_core::effect::StorageOp::Get(
                                helix_core::effect::GetSpec {
                                    table: "message",
                                    key_col: "temporary_id",
                                    key_val: helix_core::effect::SqlValue::Text(id),
                                },
                            )],
                        });
                    } else {
                        self.persist_category(work, out)?;
                    }
                }
            }
            Pending::MessageHead(mut work) => {
                let rows: Vec<Value> = serde_json::from_slice(bytes.0.as_ref())
                    .map_err(|_| invalid("message guard rows"))?;
                if rows.len() > 1 {
                    return Err(invalid("message guard identity"));
                }
                if let Some(row) = rows.first() {
                    work.post_guard = validate_message_row(row, &work)?.map(str::to_owned);
                }
                self.persist_category(work, out)?;
            }
            Pending::MessageReadback(work) => {
                let rows: Vec<Value> = serde_json::from_slice(bytes.0.as_ref())
                    .map_err(|_| invalid("message readback rows"))?;
                if rows.len() > 1 {
                    return Err(invalid("message readback identity"));
                }
                if let Some(row) = rows.first() {
                    validate_message_row(row, &work)?;
                }
                if !work.ws {
                    let row = rows
                        .first()
                        .ok_or_else(|| invalid("message readback missing"))?;
                    let props: Value = serde_json::from_str(
                        row.get("props")
                            .and_then(Value::as_str)
                            .ok_or_else(|| invalid("message readback props"))?,
                    )
                    .map_err(|_| invalid("message readback props"))?;
                    if props.get("categoryChain") != work.data.get("card") {
                        return Err(invalid("MESSAGE_CHANGED_RECONCILE"));
                    }
                }
                self.complete_category(work, out)?;
            }
            Pending::Persist(work) => {
                self.read_category(work, out);
            }
            Pending::Readback(mut work) => {
                let doc = &work.documents[work.index];
                let saved = facts::decode(bytes.0.as_ref(), &doc.revision)?;
                let Value::Object(map) = saved else {
                    return Err(invalid("readback object"));
                };
                let target = work
                    .data
                    .as_object_mut()
                    .ok_or_else(|| invalid("result object"))?;
                target.extend(map);
                if historical_reconcile(&work) {
                    if work.permissions.is_some() && work.data.get("myParticipation").is_none() {
                        let index = work
                            .documents
                            .iter()
                            .position(|document| document.value.get("myParticipation").is_some())
                            .ok_or_else(|| invalid("participation readback"))?;
                        // Permission metadata is request-local; read its independent durable snapshot before success.
                        work.index = index;
                        self.read_category(work, out);
                    } else {
                        // The receipt and optional private snapshot are the durable barriers for success.
                        self.complete_category(work, out)?;
                    }
                    return Ok(());
                }
                work.index += 1;
                if work.index < work.documents.len() {
                    self.read_category(work, out);
                } else {
                    if let Some((_, temporary_id, _)) = message_snapshot(&work) {
                        let id = temporary_id.to_owned();
                        let corr = self.alloc_corr_internal();
                        self.state.corr_map.insert(
                            corr,
                            CorrelationContext::CategoryChain {
                                pending: Box::new(Pending::MessageReadback(work)),
                            },
                        );
                        out.push(Effect::Persist {
                            corr,
                            ops: vec![helix_core::effect::StorageOp::Get(
                                helix_core::effect::GetSpec {
                                    table: "message",
                                    key_col: "temporary_id",
                                    key_val: helix_core::effect::SqlValue::Text(id),
                                },
                            )],
                        });
                    } else {
                        self.complete_category(work, out)?;
                    }
                }
            }
            Pending::Prepare { .. } | Pending::Http { .. } => {
                return Err(invalid("non-storage continuation"))
            }
        }
        Ok(())
    }

    /// 完整持久读回后才交付业务终态；authority不从客户端意图推导。
    fn complete_category(&mut self, mut work: Work, out: &mut EffectSink) -> Result<(), ImError> {
        // Permissions are request-local observations; only entries use the durable participant version.
        if let Some(permissions) = work.permissions.take() {
            let target = work
                .data
                .get_mut("myParticipation")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| invalid("participation readback"))?;
            let Value::Object(permissions) = permissions else {
                return Err(invalid("permissions readback"));
            };
            target.extend(permissions);
        }
        // A retracted public snapshot must not re-expose private data from an older receipt.
        if work.data.pointer("/card/status").and_then(Value::as_str) == Some("RETRACTED") {
            if !work.ws {
                work.data["myParticipation"] = Value::Null;
                work.data["draft"] = Value::Null;
            }
        }
        if let Some(req_id) = &work.request.req_id {
            out.push(crate::query::read_relay::emit_read_body(
                            req_id,
                            json!({"status":"SUCCESS", "data": if work.reconcile {
                            json!({"state":"CONFIRMED","clientMutationId":work.data["clientMutationId"],"operationId":work.data["operationId"],"result":work.data,"error":null})
                        } else if work.request.command == "category_chain_get_mine" { work.data["myParticipation"].take() }
                        else if super::is_read(&work.request.command) && work.request.command != "category_chain_reconcile" {work.data.take()}
                        else { work.data.clone() }}),
                        ));
        }
        if !super::is_read(&work.request.command)
            || work.request.command == "category_chain_reconcile"
            || work.ws
        {
            let fresh = work.documents.iter().any(|doc| doc.write);
            if fresh || (work.request.req_id.is_some() && !work.reconcile) {
                if !historical_reconcile(&work) {
                    if let Some(post) = work.data.get("post").filter(|p| p.is_object()) {
                        let event = post_event(post, &work.request)?;
                        let emit = if work.request.command == "category_chain_publish" {
                            crate::acl::to_effect::emit_post_received_for_viewer
                        } else {
                            crate::acl::to_effect::emit_post_updated_for_viewer
                        };
                        out.push(emit(
                            event.channel_id,
                            0,
                            &event.fields.id,
                            &event.fields,
                            &work.request.viewer,
                        ));
                    }
                }
                let mut data = work.data.take();
                data["command"] = json!(work.request.command);
                if let Some(req_id) = &work.request.req_id {
                    data["req_id"] = json!(req_id);
                }
                if let Some(id) = &work.request.mutation {
                    data["clientMutationId"] = json!(id);
                }
                let event = if work.ws { EVENT } else { STATUS_EVENT };
                if !work.ws {
                    // Private routing comes from captured context, after durable readback and read emission.
                    data["channelId"] = json!(work.request.channel);
                    data["viewerId"] = json!(work.request.viewer);
                    let state = data
                        .get("outcome")
                        .cloned()
                        .or_else(|| data.get("state").cloned())
                        .unwrap_or(Value::Null);
                    data["state"] = state;
                }
                out.push(crate::event::MessageV3Event::new(event, data)?.into_effect());
            }
        }
        self.finish_category(out)?;
        Ok(())
    }

    /// message props CAS与category事实同事务；普通消息流抢先提交时整笔回滚。
    fn persist_category(&mut self, mut work: Work, out: &mut EffectSink) -> Result<(), ImError> {
        let mut ops = Vec::new();
        for (index, doc) in work.documents.iter().enumerate() {
            if doc.write {
                ops.extend(facts::persist(&doc.scope, &doc.revision, &doc.value)?);
                if !historical_reconcile(&work) && !(work.reconcile && index == 0) {
                    if let Some(post) = doc.value.get("post").filter(|p| p.is_object()) {
                        let event = post_event(post, &work.request)?;
                        let helix_core::effect::StorageOp::BatchUpsert(mut spec) =
                            crate::channel::event_to_readback_upsert_op(&event)
                        else {
                            return Err(invalid("canonical message upsert"));
                        };
                        spec.update_guard = Some(helix_core::effect::UpsertGuard {
                            column: "props",
                            expected: work
                                .post_guard
                                .take()
                                .map(helix_core::effect::SqlValue::Text)
                                .unwrap_or(helix_core::effect::SqlValue::Null),
                        });
                        ops.push(helix_core::effect::StorageOp::BatchUpsert(spec));
                    }
                }
            }
        }
        work.index = 0;
        if ops.is_empty() {
            self.read_category(work, out);
        } else {
            let corr = self.alloc_corr_internal();
            self.state.corr_map.insert(
                corr,
                CorrelationContext::CategoryChain {
                    pending: Box::new(Pending::Persist(work)),
                },
            );
            out.push(Effect::PersistAtomic { corr, ops });
        }
        Ok(())
    }

    /// 每份成功数据从相同scope和revision的持久树恢复。
    fn read_category(&mut self, work: Work, out: &mut EffectSink) {
        let corr = self.alloc_corr_internal();
        let op = facts::read_op(
            &work.documents[work.index].scope,
            &work.documents[work.index].revision,
        );
        self.state.corr_map.insert(
            corr,
            CorrelationContext::CategoryChain {
                pending: Box::new(Pending::Readback(work)),
            },
        );
        out.push(Effect::Persist {
            corr,
            ops: vec![op],
        });
    }

    /// 成功/失败统一释放串行门，下一份authority重新读取durable版本。
    fn finish_category(&mut self, out: &mut EffectSink) -> Result<(), ImError> {
        self.state.category_chain.busy = false;
        self.with_state_and_corr_allocator(|state, alloc| start_next(state, alloc, out))
    }
}

/// 只把状态/错误和关联发回调用者；不含伪造的Card、人数或权限。
fn fail(request: &Request, state: &str, code: &str, out: &mut EffectSink) -> Result<(), ImError> {
    if let Some(req_id) = &request.req_id {
        out.push(crate::query::read_relay::emit_read_error(req_id, code));
    }
    if let Some(id) = &request.mutation {
        out.push(
            crate::event::MessageV3Event::new(
                STATUS_EVENT,
                json!({
                    "channelId":request.channel,"viewerId":request.viewer,"chainId":request.chain,"clientMutationId":id,
                    "command":request.command,"state":state,"errorCode":code
                }),
            )?
            .into_effect(),
        );
    }
    Ok(())
}

/// Locate the same anchor identity for either a canonical Post or its paired public WS card.
fn message_snapshot(work: &Work) -> Option<(&str, &str, &str)> {
    for doc in &work.documents {
        if let Some(post) = doc.value.get("post").filter(|p| p.is_object()) {
            return Some((
                post.get("id")?.as_str()?,
                post.get("temporaryId")?.as_str()?,
                &doc.revision,
            ));
        }
    }
    if work.ws {
        let root = &work.documents.first()?.value;
        return Some((
            root.get("anchorPostId")?.as_str()?,
            root.get("temporaryId")?.as_str()?,
            root.get("revision")?.as_str()?,
        ));
    }
    None
}

/// Reject stale or mismatched snapshots against the independently persisted ordinary message flow.
fn validate_message_row<'a>(row: &'a Value, work: &Work) -> Result<Option<&'a str>, ImError> {
    let (id, _, incoming_revision) =
        message_snapshot(work).ok_or_else(|| invalid("message document"))?;
    if row.get("channel_id").and_then(Value::as_str) != Some(work.request.channel.as_str()) {
        return Err(invalid("message channel mismatch"));
    }
    if row
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.is_empty() && kind != "CATEGORY_CHAIN")
    {
        return Err(invalid("message type mismatch"));
    }
    if row
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .is_some_and(|current| current != id)
    {
        return Err(invalid("message identity mismatch"));
    }
    let props = row.get("props").and_then(Value::as_str);
    if let Some(props) = props.filter(|p| !p.is_empty()) {
        let current: Value = serde_json::from_str(props).map_err(|_| invalid("message props"))?;
        let revision = current
            .pointer("/categoryChain/revision")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("message category revision"))?;
        if facts::revision_cmp(revision, incoming_revision)? == std::cmp::Ordering::Greater {
            return Err(invalid("STALE_MESSAGE_RECONCILE"));
        }
    }
    Ok(props)
}

/// 单事务public快照与本人state分开存储；receipt保留mutation关联，版本不经浮点。
pub(crate) fn make_work(
    request: Request,
    mut data: Value,
    event_seq: Option<crate::state::Seq>,
    ws: bool,
) -> Result<Work, ImError> {
    let reconcile = request.command == "category_chain_reconcile"
        && data.get("state").and_then(Value::as_str) == Some("CONFIRMED");
    if reconcile {
        data = data
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(|| invalid("reconcile result"))?;
    }
    let receipt_data = reconcile.then(|| data.clone());
    let chain = string(&data, "chainId")
        .or_else(|| data.get("chain").and_then(|c| string(c, "id")))
        .or_else(|| data.get("card").and_then(|c| string(c, "chainId")))
        .or_else(|| request.chain.to_owned())
        .unwrap_or_default();
    let visibility_scope = json!([request.viewer, request.channel, chain, "retracted"]).to_string();
    let mut documents = Vec::new();
    let mut permissions = None;
    if request.command == "category_chain_get_mine" || request.command == "category_chain_entries" {
        let version_key = if request.command == "category_chain_get_mine" {
            "participationRevision"
        } else {
            "categoryRevision"
        };
        let revision = string(&data, version_key).ok_or_else(|| invalid(version_key))?;
        if request.command == "category_chain_get_mine" {
            permissions = Some(participation_document(
                &request,
                &chain,
                data,
                &mut documents,
            )?);
        } else {
            let scope = json!([
                request.viewer,
                request.channel,
                chain,
                request.command,
                request.query_scope
            ])
            .to_string();
            documents.push(Document {
                scope,
                revision,
                value: data,
                write: true,
            });
        }
        return Ok(Work {
            request,
            visibility_scope,
            post_guard: None,
            permissions,
            documents,
            index: 0,
            data: json!({}),
            event_seq,
            ws,
            reconcile,
        });
    }
    let root = data
        .as_object_mut()
        .ok_or_else(|| invalid("authority object"))?;
    let public_revision = root
        .get("card")
        .and_then(|v| string(v, "revision"))
        .or_else(|| root.get("chain").and_then(|v| string(v, "revision")));
    // Card-only WS cannot replace HTTP chain/Post fields; each fact has its own durable head.
    for key in ["card", "chain", "post", "anchorPost"] {
        if root.get(key).is_some_and(|v| !v.is_null()) {
            let value = root.remove(key).ok_or_else(|| invalid(key))?;
            let revision = public_revision
                .as_ref()
                .ok_or_else(|| invalid("public revision"))?
                .to_owned();
            let scope = json!([request.viewer, request.channel, chain, key]).to_string();
            documents.push(Document {
                scope,
                revision,
                value: json!({key:value}),
                write: true,
            });
        }
    }
    if root.get("myParticipation").is_some_and(|v| !v.is_null()) {
        let mine = root
            .remove("myParticipation")
            .ok_or_else(|| invalid("myParticipation"))?;
        permissions = Some(participation_document(
            &request,
            &chain,
            mine,
            &mut documents,
        )?);
    }
    if let Some(revision) = documents
        .iter()
        .find(|d| d.value.pointer("/card/status").and_then(Value::as_str) == Some("RETRACTED"))
        .map(|d| d.revision.to_owned())
    {
        // Terminal visibility is keyed separately from personal revisions; no invented max version.
        documents.push(Document {
            scope: visibility_scope.to_owned(),
            revision,
            value: json!({}),
            write: true,
        });
    }
    let receipt_id = request
        .mutation
        .as_deref()
        .unwrap_or(request.query_scope.as_str());
    let revision = root
        .get("revision")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| documents.first().map(|d| d.revision.to_owned()))
        .unwrap_or_else(|| "0".to_owned());
    let scope = json!([
        request.viewer,
        request.channel,
        chain,
        request.command,
        receipt_id,
        if request.command == "category_chain_reconcile" {
            root.get("state")
                .and_then(Value::as_str)
                .unwrap_or("CONFIRMED")
        } else {
            ""
        }
    ])
    .to_string();
    documents.insert(
        0,
        Document {
            scope,
            revision,
            value: receipt_data.unwrap_or(data),
            write: true,
        },
    );
    Ok(Work {
        request,
        visibility_scope,
        post_guard: None,
        permissions,
        documents,
        index: 0,
        data: json!({}),
        event_seq,
        ws,
        reconcile,
    })
}

/// Version entries separately from request-scoped permissions, which can change without an entry edit.
fn participation_document(
    request: &Request,
    chain: &str,
    mut mine: Value,
    documents: &mut Vec<Document>,
) -> Result<Value, ImError> {
    let revision =
        string(&mine, "participationRevision").ok_or_else(|| invalid("participationRevision"))?;
    let fields = mine
        .as_object_mut()
        .ok_or_else(|| invalid("myParticipation"))?;
    let mut permissions = serde_json::Map::new();
    for key in ["canParticipate", "canEdit", "canCancel", "denialReason"] {
        permissions.insert(
            key.to_owned(),
            fields.remove(key).ok_or_else(|| invalid(key))?,
        );
    }
    documents.push(Document {
        scope: json!([request.viewer, request.channel, chain, "mine"]).to_string(),
        revision: revision.clone(),
        value: json!({"myParticipation":mine}),
        write: true,
    });
    Ok(Value::Object(permissions))
}

/// 同一模块中的分类写集串行，避免两个head读取后以旧revision覆盖新快照。
pub(crate) fn enqueue(
    state: &mut ImState,
    alloc: &mut dyn FnMut() -> Correlation,
    work: Work,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    if state.category_chain.queue.len() >= 64 {
        return fail(&work.request, "RECONCILING", "CATEGORY_QUEUE_FULL", out);
    }
    state.category_chain.queue.push_back(work);
    start_next(state, alloc, out)
}

/// 队首重新验证频道seq与durable head；缺口走已有sync调度。
fn start_next(
    state: &mut ImState,
    alloc: &mut dyn FnMut() -> Correlation,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    if state.category_chain.busy {
        return Ok(());
    }
    while let Some(mut work) = state.category_chain.queue.pop_front() {
        if let Some(seq) = work.event_seq {
            let id = crate::state::ChannelId::from_str(&work.request.channel)
                .ok_or_else(|| invalid("channelId"))?;
            let channel = state
                .channels
                .entry(id)
                .or_insert_with(|| crate::channel::Channel::new(id, 0));
            if seq <= channel.cursor.value() {
                work.event_seq = None;
            } else if !channel.admit_chain_event_seq(seq, out) {
                continue;
            }
            // The paired canonical Post owns the cursor commit; a card alone cannot acknowledge it.
            work.event_seq = None;
        }
        let op = facts::head_op(&work.visibility_scope);
        let corr = alloc();
        state.corr_map.insert(
            corr,
            CorrelationContext::CategoryChain {
                pending: Box::new(Pending::Visibility(work)),
            },
        );
        state.category_chain.busy = true;
        out.push(Effect::Persist {
            corr,
            ops: vec![op],
        });
        break;
    }
    Ok(())
}

/// 转为可追踪的业务边界错误，不暴露凭据或原始payload。
fn invalid(field: &str) -> ImError {
    ImError::Parse(format!("CATEGORY_CHAIN_INVALID: {field}"))
}

/// 只在公共 authority 明确高于 receipt 时关闭旧的 Post 写入与事件。
fn historical_reconcile(work: &Work) -> bool {
    work.reconcile && work.post_guard.as_deref() == Some(HISTORICAL_RECONCILE_GUARD)
}

/// 将完整canonical Post复用现有presence-aware message落库与EventSink工厂。
fn post_event(
    post: &Value,
    request: &Request,
) -> Result<crate::sync_session::EventEnvelope, ImError> {
    let channel = crate::state::ChannelId::from_str(&request.channel)
        .ok_or_else(|| invalid("post channel"))?;
    let fields = crate::ws::parser::extract_post_fields(post);
    if fields.id.is_empty()
        || fields.temporary_id.is_empty()
        || fields.msg_type != "CATEGORY_CHAIN"
        || fields.channel_id != request.channel
    {
        return Err(invalid("canonical Post identity/type"));
    }
    Ok(crate::sync_session::EventEnvelope::new(
        channel,
        crate::state::Seq(0),
        crate::sync_session::EventKind::PostUpsert,
        fields,
    ))
}
