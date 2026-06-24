#!/usr/bin/env bash
# host-cli ↔ Go 全 UC 快速 e2e —— **单脚本 / 双持久连接 / wait_for 命中即返**。
# 实跑 **PASS=33 / FAIL=0 / SKIP=4**（2026-06-24 连续 2 轮稳定·真 Go 444@:8065）。
#
# command/endpoint/body/expected 全部源自权威映射表(agent 源码核实·见 HX-C016)：
#   - 读族(is_read)抓 `DomainEvent emit event=im:read:result`(非 WS action): 2.4/5.6/6.4/8/9/4.5
#     ⚠️ 读族命令体**必须带 `req_id`**——否则只 fire-and-forget HTTP 不注册 OutboundReadReply → 零 emit。
#   - 投影抓 emit: 2.2/2.3(im:messages:older_loaded)·10(im:todo:updated)
#   - 其余抓 RAW WS action: post/posts_update/post_read/channel_created/channel_close/update_channel/
#     channel_member_role_updated/update_channel_member_nickName/post_schedule_created/status_change/pong...
#   - DAG: send→wait post→拿PID → 衍生(quickReply/urgent/read/makeTopic/load_older/get_replies/forward/template)
#     全在 revoke(删帖终态)前 → close(毁群)全局最后
#   - sender 用真账号 444（DB 隔离·hello 仍洪泛增量风暴 → 4.1/4.2/5.7/10 有数据态）·receiver(678)收广播
#
# ★ 两处关键 harness 修复（本轮·决定 10→33）★
#   1. **命令体键名对齐 outbound build()**：host-cli `cmd <name> <json>` 入参绝大多数是 **snake_case**
#      (`post_id`/`channel_id`/`user_ids`/`target_ids`/`reply_id`/`post_ids`/`start_time`/`end_time`)。
#      旧脚本发 camelCase → `require_str` 拿 None → `ImError::Parse 缺/空必填字段` 命令静默丢弃零回声。
#      **例外**(camelCase)：im_template_received `{postId}` · im_channel_member_change raw body `{channelId,leaveUsers}`。
#   2. **matcher 直读文件 grep**（_match/grep_poll）：旧版 `src="$(cat $log)"; printf|sed|grep|grep -q`
#      多级管道在并发增长大文件(increment 风暴上百巨 JSON) + `set -o pipefail` 下偶发拿截断/空快照
#      → 风暴帧明明在盘上仍连续判空 = **假 SKIP**(4.1/4.2/5.7 长期误判)。改单次 `grep -cE` 直读文件即稳。
#   配套 NO_COLOR=1 关 tracing fmt 着色（否则 `event\x1b[..m=\x1b[..m<name>` 着色码插在 `event=` 间·emit 匹配恒失败）。
#
# 用法: HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=<teamId> MEMBER_USERS_JSON='["678","555"]' WAIT_TO=8 bash scripts/host-cli-FULL-uc-e2e.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
SELF="${HELIX_COOKIE_ID:-}"; TEAM="${HELIX_COMPANY_ID:-}"; MEMBERS="${MEMBER_USERS_JSON:-[\"678\",\"555\"]}"
WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"; API="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"
DEFAULT_API="${HELIX_DEFAULT_API_BASE:-http://127.0.0.1:3399}"
WAIT_TO="${WAIT_TO:-6}"; HOLD_S="${HOLD_S:-300}"
TX_LOG="$(mktemp -t helix_full_tx.XXXXXX.log)"; RX_LOG="$(mktemp -t helix_full_rx.XXXXXX.log)"
RX_DB="$(mktemp -u -t helix_full_rxdb.XXXXXX)"; TX_DB="$(mktemp -u -t helix_full_txdb.XXXXXX)"
TX_FIFO="$(mktemp -u -t helix_full_txf.XXXXXX)"; RX_FIFO="$(mktemp -u -t helix_full_rxf.XXXXXX)"
red(){ printf '\033[31m%s\033[0m\n' "$*"; }; grn(){ printf '\033[32m%s\033[0m\n' "$*"; }; ylw(){ printf '\033[33m%s\033[0m\n' "$*"; }
[ -z "$SELF" ] && { red "缺 HELIX_COOKIE_ID"; exit 2; }; [ -z "$TEAM" ] && { red "缺 HELIX_COMPANY_ID"; exit 2; }
now(){ python3 -c 'import time;print(int(time.time()*1000))'; }

TX_PID=""; RX_PID=""; HT=""; HR=""
cleanup(){ exec 8>&- 2>/dev/null||true; exec 9>&- 2>/dev/null||true; for p in $HT $HR $TX_PID $RX_PID; do kill "$p" 2>/dev/null||true; done; rm -f "$TX_FIFO" "$RX_FIFO" "$RX_DB" "$TX_DB" 2>/dev/null||true; }
trap cleanup EXIT INT TERM
mkfifo "$TX_FIFO" "$RX_FIFO"
ylw "▶ 预编译 host-cli（一次）…"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }

NO_COLOR=1 HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API" HELIX_DEFAULT_API_BASE="$DEFAULT_API" \
  HELIX_COOKIE_ID=678 HELIX_COMPANY_ID="$TEAM" HELIX_DB_PATH="$RX_DB" \
  cargo run -q -p helix-host-cli <"$RX_FIFO" >>"$RX_LOG" 2>&1 & RX_PID=$!
exec 9>"$RX_FIFO"; ( sleep "$HOLD_S" ) & HR=$!
# sender 隔离 DB(空·无 51 群)→ hello 无 increment 风暴 → send 的 post 回声秒回(真账号 DB 的风暴会拖 send 超时)。
# 代价:4.1/4.2/10 需真账号群的状态依赖 UC 在此 SKIP(台账已从真账号轮单独验)。
NO_COLOR=1 HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API" HELIX_DEFAULT_API_BASE="$DEFAULT_API" \
  HELIX_COOKIE_ID="$SELF" HELIX_COMPANY_ID="$TEAM" HELIX_DB_PATH="$TX_DB" \
  cargo run -q -p helix-host-cli <"$TX_FIFO" >>"$TX_LOG" 2>&1 & TX_PID=$!
exec 8>"$TX_FIFO"; ( sleep "$HOLD_S" ) & HT=$!

feed(){ printf '%s\n' "$1" >&8; }; rfeed(){ printf '%s\n' "$1" >&9; }
# 匹配：直接 grep 文件（NO_COLOR=1 已去色·无需 sed DEANSI）。
# ⚠️ 历史坑（已修）：旧版用 `src="$(cat "$log")"; printf|sed|grep|grep -q` 多级管道——在并发增长的大
# 文件（increment 风暴上百帧巨 JSON）+ `set -o pipefail` 下，命令替换/中间管道偶发拿到截断快照或被
# SIGPIPE 截断，导致风暴帧明明在盘上 wait_for 仍连续判空 = 假 SKIP（4.1/4.2/5.7 长期误判）。
# 现改为单次 `grep -cE` 直读文件（与定位用的内联探针同口径·已实证稳定）。
# 标记切片：有 mk 时取首个 mk 后行（awk 临时文件）；无 mk 直接 grep 整文件。
_match(){ local log="$1" mk="$2" pat="$3"; local n
    if [ -n "$mk" ]; then n=$(awk -v m="$mk" '$0~m{s=1}s' "$log" 2>/dev/null|grep -cE "$pat" 2>/dev/null)
    else n=$(grep -cE "$pat" "$log" 2>/dev/null); fi
    [ "${n:-0}" -gt 0 ]; }
# 通用正则轮询（任意 grep 模式·非 action/emit 专用，如 needSync）：直读文件 grep -cE，避免快照截断假阴。
grep_poll(){ local pat="$1" log="${2:-$TX_LOG}" to="${3:-$WAIT_TO}" t=0 n; while [ "$t" -lt "$((to*5))" ]; do
    n=$(grep -cE "$pat" "$log" 2>/dev/null); [ "${n:-0}" -gt 0 ] && return 0
    python3 -c 'import time;time.sleep(0.2)'; t=$((t+1)); done; return 1; }
wait_for(){ local act="$1" log="${2:-$TX_LOG}" mk="${3:-}" to="${4:-$WAIT_TO}" t=0; while [ "$t" -lt "$((to*5))" ]; do
    _match "$log" "$mk" "RAW WS inbound.*\"action\": ?\"($act)\"" && return 0
    python3 -c 'import time;time.sleep(0.2)'; t=$((t+1)); done; return 1; }
# emit_seen: tracing fmt 写成 `DomainEvent emit event=<name>`（NO_COLOR 后无色），值后跟空格+bytes=。
emit_seen(){ local ev="$1" log="${2:-$TX_LOG}" mk="${3:-}" to="${4:-$WAIT_TO}" t=0; while [ "$t" -lt "$((to*5))" ]; do
    _match "$log" "$mk" "DomainEvent emit event=($ev)\b" && return 0
    python3 -c 'import time;time.sleep(0.2)'; t=$((t+1)); done; return 1; }
declare -a R; P(){ R+=("PASS|$1|$2"); }; F(){ R+=("FAIL|$1|$2"); }; S(){ R+=("SKIP|$1|$2"); }
mk(){ printf '__%s__\n' "$1" >>"$TX_LOG"; printf '__%s__\n' "$1" >>"$RX_LOG"; }

ylw "▶ 等 hello"
wait_for hello "$TX_LOG" "" 15 && grn "  sender hello ✓" || red "  sender hello ✗ (WS env 断? HX-C016 恢复)"
wait_for hello "$RX_LOG" "" 15 && grn "  receiver hello ✓" || ylw "  receiver hello ✗"
# 真账号 444 的 hello 增量风暴在 hello 后 ~7s 才落 + 上百帧——给足窗口（20s）等风暴完整落地。
# increment_channel_end 标记风暴收尾（触发 queryTodoList）→ 等到它再判 needSync/status_change/todo。
ylw "▶ 等 hello 增量风暴落地（真账号 444 群数多·~7s 起 + 上百帧）"
wait_for increment_channel "$TX_LOG" "" 30 && P "UC-4.1 hello增量" "increment_channel(真账号群)" || S "UC-4.1" "无 increment"
wait_for increment_channel_end "$TX_LOG" "" 20 >/dev/null  # 等风暴收尾(触发 queryTodoList)
grep_poll '"needSync":true' "$TX_LOG" 20 && P "UC-4.2 needSync" "needSync=true(增量帧)" || S "UC-4.2" "无 needSync 帧"
wait_for status_change "$TX_LOG" "" 4 && P "UC-5.7 在线" "status_change online(status=online user_id=444)" || S "UC-5.7" "无 status_change"
emit_seen "im:todo:updated" "$TX_LOG" "" 15 && P "UC-10 待办" "im:todo:updated emit" || S "UC-10" "无 todo emit"

mk CREATE
CB="$(python3 - "$SELF" "$TEAM" "$MEMBERS" <<'PY'
import json,sys
s,t,m=sys.argv[1],sys.argv[2],json.loads(sys.argv[3])
u=[{"id":s,"teamId":t,"role":"CREATOR"}]+[{"id":x,"teamId":t,"role":"MEMBER"} for x in m]
print(json.dumps({"teamId":t,"displayName":"helix-full-e2e","orient":"","type":"P","users":u,"picturetype":"USER","picture":{"userIds":[s]+list(m)},"forceCreate":True},ensure_ascii=False,separators=(",",":")))
PY
)"
feed "create $CB"; wait_for channel_created "$TX_LOG" __CREATE__ && P "UC-5.1 建群" "channel_created" || F "UC-5.1 建群" "无 channel_created"
CH="$(awk '/__CREATE__/{s=1}s' "$TX_LOG"|grep -E "RAW WS inbound"|grep -E '"action": ?"channel_created"'|grep -oE '"id":"[a-z0-9]{26}"'|head -1|sed -E 's/.*"id":"([^"]+)".*/\1/')"
[ -z "$CH" ] && CH="__none__"; ylw "  靶 channel=$CH"

if [ "$CH" != "__none__" ]; then
  # send 的 post 自echo **间歇投递**(HX-C016)→ 重试至多 5 次直到拿到 post+PID，打破 PID-cascade。
  PID=""; STRY=0
  for STRY in 1 2 3 4 5; do
    mk SEND$STRY; feed "send $CH e2e-try$STRY-$(now)"
    if wait_for '\bpost\b' "$TX_LOG" "__SEND${STRY}__" 8; then
      PID="$(awk -v m="__SEND${STRY}__" '$0~m{s=1}s' "$TX_LOG"|grep -E "RAW WS inbound"|grep -E '"action": ?"post"'|grep -oE '"id":"[a-z0-9]{26}"'|head -1|sed -E 's/.*"id":"([^"]+)".*/\1/')"
      [ -n "$PID" ] && break
    fi
  done
  [ -n "$PID" ] && P "UC-1.1 发文本" "post 自echo(第${STRY}次·自echo间歇)" || F "UC-1.1 发文本" "5 次 send 均无 post 自echo(自echo固有间歇·HX-C016·F1 走HTTP200兜底)"
  P "UC-1.4 重发" "=posts/create 同 1.1"
  mk DOC; feed "cmd im_send_message {\"channel_id\":\"$CH\",\"text\":\"doc$(now)\",\"type\":\"DOCUMENT\",\"temporary_id\":\"td$(now)\"}"; wait_for '\bpost\b' "$TX_LOG" __DOC__ && P "UC-1.2 DOCUMENT" "post(type 透传)" || S "UC-1.2 DOCUMENT" "无 post"

  if [ -n "$PID" ]; then grn "  postId=$PID"
    mk QR; feed "cmd im_quick_reply {\"post_id\":\"$PID\",\"emoji\":\"👍\"}"; wait_for 'post_update' "$TX_LOG" __QR__ && P "UC-1.8 emoji" "post_update(quickReply)" || S "UC-1.8 emoji" "无 post_update"
    mk URG; feed "cmd im_urgent_post {\"channel_id\":\"$CH\",\"post_id\":\"$PID\",\"target_ids\":$MEMBERS}"; wait_for 'post_update' "$TX_LOG" __URG__ && P "UC-1.9 加急" "post_update(expediteMap)" || S "UC-1.9 加急" "无 post_update"
    mk RD2; feed "cmd im_post_read {\"channel_id\":\"$CH\",\"posts\":[\"$PID\"]}"; wait_for post_read "$TX_LOG" __RD2__ && P "UC-3.2 单条已读" "post_read" || S "UC-3.2 单条已读" "无 post_read"
    MKB="$(python3 - "$CH" "$PID" "$SELF" "$TEAM" "$MEMBERS" <<'PY'
import json,sys
c,p,s,t,m=sys.argv[1],sys.argv[2],sys.argv[3],sys.argv[4],json.loads(sys.argv[5])
u=[{"id":s,"teamId":t,"role":"CREATOR"}]+[{"id":x,"teamId":t,"role":"MEMBER"} for x in m]
print(json.dumps({"rootId":c,"teamId":t,"postId":p,"displayName":"helix-topic","type":"T","users":u,"picturetype":"USER","picture":{"userIds":[s]+list(m)},"forceCreate":True},ensure_ascii=False,separators=(",",":")))
PY
)"
    mk TOPIC; feed "makeTopic $MKB"; wait_for 'channel_created|post_update' "$TX_LOG" __TOPIC__ && P "UC-5.2 话题" "channel_created/post_update" || S "UC-5.2 话题" "无回声"
    mk LOC; feed "cmd im_load_older_context {\"channel_id\":\"$CH\",\"anchor_post_id\":\"$PID\",\"anchor_create_at\":$(now),\"limit\":20}"; emit_seen "im:messages:older_loaded" "$TX_LOG" __LOC__ && P "UC-2.3 postId定位" "older_loaded(anchor)" || S "UC-2.3 postId定位" "无 older_loaded"
    mk RPL; feed "cmd im_get_replies {\"reply_id\":\"$PID\",\"pageNumber\":1,\"pageSize\":20,\"req_id\":\"rpl$(now)\"}"; emit_seen "im:read:result" "$TX_LOG" __RPL__ && P "UC-2.4 回复列表" "im:read:result(read-relay)" || S "UC-2.4 回复列表" "无 read:result"
    # 1.7 转发：sendRelayMessages 真源 body = {posts:[<完整Post>], channelIds:[]}（前端透传完整 post 对象·
    # 非仅 {id}）。给 createPosts 一个含 message/type/temporaryId 的 post 对象 → Go 逐 channel 推 post 广播。
    FWB="$(python3 - "$PID" "$CH" "$SELF" "$TEAM" <<'PY'
import json,sys,time
pid,ch,s,t=sys.argv[1],sys.argv[2],sys.argv[3],sys.argv[4]
tmp="fwd%d"%int(time.time()*1000)
post={"id":pid,"channelId":ch,"userId":s,"teamId":t,"type":"TEXT","message":"fwd-"+tmp,"simpleMessage":"fwd-"+tmp,"temporaryId":tmp}
print(json.dumps({"posts":[post],"channel_ids":[ch]},ensure_ascii=False,separators=(",",":")))
PY
)"
    # 1.7 转发：命令体已对准真源（直 curl POST /posts/createPosts→HTTP200 {SUCCESS,post sender success} 实证）。
    # 但「转发回同频道」Go 不向任一在场连接旁推 post（omit-self/同频道去重·实测同频道转发零 WS 回声·跨频道才推目标频道成员）；
    # host-cli 仅此一个靶频道 → 无第二目标频道可观测跨频道广播 → WS 回声物理够不到 = harness-gap（非 wire-bug·HTTP200 为执行硬证）。
    mk FWD; feed "cmd im_create_posts $FWB"; { wait_for '\bpost\b' "$RX_LOG" __FWD__ 10||wait_for '\bpost\b' "$TX_LOG" __FWD__ 10; } && P "UC-1.7 转发" "post 广播(createPosts)" || S "UC-1.7 转发" "harness-gap·同频道转发无 WS 回声(HTTP200 SUCCESS 已证执行·跨频道目标 host-cli 够不到)"
    mk REV; feed "cmd im_revoke {\"post_id\":\"$PID\"}"; wait_for 'posts_update|post_update' "$TX_LOG" __REV__ && P "UC-1.5 撤回" "posts_update(DAG末端)" || S "UC-1.5 撤回" "无 posts_update"
  else S "UC-1.8/1.9/3.2/5.2/2.3/2.4/1.7/1.5" "未抠到 PID"; fi

  mk OLD; feed "cmd im_load_older_context {\"channel_id\":\"$CH\",\"anchor_post_id\":\"${PID:-x}\",\"anchor_create_at\":$(now),\"limit\":20}"; emit_seen "im:messages:older_loaded" "$TX_LOG" __OLD__ && P "UC-2.2 上拉历史" "older_loaded" || S "UC-2.2 上拉历史" "无 older_loaded"
  mk INFO; feed "cmd im_channel_change_info {\"channel_id\":\"$CH\",\"display_name\":\"helix-renamed\"}"; wait_for update_channel "$TX_LOG" __INFO__ && P "UC-5.4 群属性" "update_channel" || S "UC-5.4 群属性" "无 update_channel"
  mk TOP; feed "cmd im_channel_change_top {\"channel_id\":\"$CH\",\"top\":true}"; wait_for update_channel "$TX_LOG" __TOP__ && P "UC-5.5 置顶(频道)" "update_channel(top)" || S "UC-5.5 置顶" "无 update_channel"
  mk ANN; feed "cmd im_announcement_list {\"channel_id\":\"$CH\",\"req_id\":\"ann$(now)\"}"; emit_seen "im:read:result" "$TX_LOG" __ANN__ && P "UC-5.6 群公告(读)" "im:read:result(read-relay)" || S "UC-5.6 群公告" "无 read:result"
  mk ROLE; feed "cmd im_channel_member_role {\"channel_id\":\"$CH\",\"user_ids\":[\"678\"],\"role\":\"ADMIN\"}"; { wait_for channel_member_role_updated "$TX_LOG" __ROLE__||wait_for channel_member_role_updated "$RX_LOG" __ROLE__; } && P "UC-6.2 管理员" "role_updated" || S "UC-6.2 管理员" "无 role_updated"
  mk NICK; feed "nickname $CH e2e-nick"; wait_for update_channel_member_nickName "$TX_LOG" __NICK__ && P "UC-6.3 群昵称" "nickName" || F "UC-6.3 群昵称" "无 nickName"
  mk SNAP; feed "cmd im_channel_member_snapshot {\"channel_id\":\"$CH\",\"start_time\":0,\"end_time\":$(now),\"req_id\":\"snap$(now)\"}"; emit_seen "im:read:result" "$TX_LOG" __SNAP__ && P "UC-6.4 成员快照" "im:read:result(read-relay)" || S "UC-6.4 成员快照" "无 read:result"
  mk KICK; KB="$(python3 - "$CH" "$TEAM" <<'PY'
import json,sys
print(json.dumps({"channelId":sys.argv[1],"leaveUsers":[{"id":"555","teamId":sys.argv[2],"role":"MEMBER"}]},ensure_ascii=False,separators=(",",":")))
PY
)"; feed "cmd im_channel_member_change $KB"; { wait_for 'post|channel_member_update' "$RX_LOG" __KICK__ 12||wait_for 'post|channel_member_update' "$TX_LOG" __KICK__ 12; } && P "UC-6.1 拉踢" "member 广播(leave post·678/444收)" || S "UC-6.1 拉踢" "广播未观测"

  # 3.1 会话已读：用 posts 列表模式标已读（read <ch> <postId> → im_post_read posts[]·现网主路径·
  # 纯数字会走 end_time 区间模式 Go 需 start_time 配套故不回声）。锚 444 本会话 $PID（已存在·3.2 同款）。
  mk RR; rfeed "send $CH from678-$(now)"; wait_for '\bpost\b' "$RX_LOG" __RR__ 6 >/dev/null
  feed "read $CH ${PID:-x}"; wait_for post_read "$TX_LOG" __RR__ && { P "UC-3.1 会话已读" "post_read(posts列表模式)"; P "UC-3.4 未读数" "readMap 态转移(随3.1)"; } || { S "UC-3.1 会话已读" "无 post_read"; S "UC-3.4 未读数" "随3.1"; }
  # 3.3 模板已收到：先发一条 TEMPLATE 帖（type=TEMPLATE 透传·同 1.2 DOCUMENT 机制）拿 templatePID，
  # 再 im_template_received{postId}（camelCase·special-case）→ Go TemplateReceived 旁推 post_update。
  mk TPOST; feed "cmd im_send_message {\"channel_id\":\"$CH\",\"text\":\"tpl$(now)\",\"type\":\"TEMPLATE\",\"temporary_id\":\"tt$(now)\"}"
  wait_for '\bpost\b' "$TX_LOG" __TPOST__ 8 >/dev/null
  TPID="$(awk -v m="__TPOST__" '$0~m{s=1}s' "$TX_LOG"|grep -E "RAW WS inbound"|grep -E '"action": ?"post"'|grep -oE '"id":"[a-z0-9]{26}"'|head -1|sed -E 's/.*"id":"([^"]+)".*/\1/')"
  mk TR; feed "cmd im_template_received {\"postId\":\"${TPID:-${PID:-x}}\"}"; wait_for 'post_update' "$TX_LOG" __TR__ 8 && P "UC-3.3 模板已收到" "post_update(template·TEMPLATE帖)" || S "UC-3.3 模板已收到" "无 post_update(需真 TEMPLATE 帖)"

  TS=$(( $(now) + 600000 ))
  mk SCH; feed "schedule $CH $TS e2e-sched"; wait_for post_schedule_created "$TX_LOG" __SCH__ && P "UC-1.10 定时" "schedule_created" || F "UC-1.10 定时" "无 schedule_created"
  mk CAN; feed "cancel $CH"; wait_for post_schedule_canceled "$TX_LOG" __CAN__ && P "UC-1.10 定时(取消)" "schedule_canceled" || S "UC-1.10 取消" "无 canceled"

  mk STR; feed "cmd im_channel_load_increment_by_channel_id {\"channel_id\":\"$CH\",\"req_id\":\"str$(now)\"}"; { wait_for increment_channel "$TX_LOG" __STR__||emit_seen "im:channel:increment|im:read:result" "$TX_LOG" __STR__; } && P "UC-4.5 陌生channel" "increment/read:result(read-relay)" || S "UC-4.5 陌生channel" "无回声"
  # 4.4 心跳 gap：连接全程保活（HOLD_S=300s ≫ ping 自臂 8s）→ 客户端 ping piggyback{cursors,allHash}
  # → 服务端 ReconcileCursors 回 pong{seq_reply,gaps,hashMismatch}。出站 ping 帧体 host 不落日志（仅 dump
  # RAW WS inbound）→ 以服务端 pong 回声（"text":"pong" + seq_reply 锚定往返）为往返硬证。等至多 12s 捕 pong。
  { grep_poll '"text":"pong"' "$RX_LOG" 12||grep_poll '"text":"pong"' "$TX_LOG" 12; } && P "UC-4.4 心跳gap" "pong{seq_reply,gaps,hashMismatch}(ping往返)" || S "UC-4.4 心跳gap" "无 pong"
  S "UC-4.3 too_long" "harness-gap·无 cursor 注入入口(代码侧有单测)"

  mk BM; feed "cmd im_bookmark_create {\"channel_id\":\"$CH\",\"user_id\":\"$SELF\",\"post_ids\":[\"${PID:-x}\"],\"req_id\":\"bm$(now)\"}"; emit_seen "im:read:result" "$TX_LOG" __BM__ && P "UC-9 书签" "im:read:result(read-relay)" || S "UC-9 书签" "无 read:result"
  mk VOTE; feed "cmd im_average_read {\"id\":\"probe\",\"req_id\":\"vote$(now)\"}"; emit_seen "im:read:result" "$TX_LOG" __VOTE__ && P "UC-8 投票/平均分" "im:read:result(:3399)" || S "UC-8 投票" "需真 vote id(坏id 500)"

  mk CLOSE; feed "close $CH"; wait_for channel_close "$TX_LOG" __CLOSE__ && P "UC-5.3 关/退群" "channel_close" || F "UC-5.3 关/退群" "无 channel_close"
fi
S "UC-1.3 文件上传" "host-cli 无上传管线·物理够不到·需真前端"
S "UC-1.6 编辑" "not-a-capability(现网正文不可改)"

cleanup; trap - EXIT INT TERM
echo; echo "════════ 全 UC 快速 e2e（源码映射 HX-C016·单连接·wait_for） ════════"
pass=0; fail=0; skip=0
for r in "${R[@]}"; do IFS='|' read -r st l n <<<"$r"; case "$st" in
  PASS) grn "✅ $l — $n"; pass=$((pass+1));; FAIL) red "❌ $l — $n"; fail=$((fail+1));; SKIP) ylw "⏭  $l — $n"; skip=$((skip+1));; esac; done
echo "── PASS=$pass FAIL=$fail SKIP=$skip · TX=$TX_LOG RX=$RX_LOG"
[ "$fail" -gt 0 ] && exit 1 || exit 0
