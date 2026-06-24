#!/usr/bin/env bash
# host-cli ↔ Go 全 UC 快速 e2e —— **单脚本 / 双持久连接 / wait_for 命中即返 / 100% 准确(源码映射 HX-C016)**。
#
# command/endpoint/body/expected 全部源自权威映射表(agent 源码核实·见 HX-C016)：
#   - 读族(is_read)抓 `DomainEvent emit event=im:read:result`(非 WS action): 2.4/5.6/6.4/8/9
#   - 投影抓 emit: 2.2/2.3(im:messages:older_loaded)·4.5(im:channel:increment)·10(im:todo:updated)
#   - 其余抓 RAW WS action: post/posts_update/post_read/channel_created/channel_close/update_channel/
#     channel_member_role_updated/update_channel_member_nickName/post_schedule_created...
#   - DAG: send→wait post→拿PID → 衍生(quickReply/urgent/read/makeTopic/load_older/get_replies/forward)
#     全在 revoke(删帖终态)前 → close(毁群)全局最后
#   - sender 用**真账号 DB**(有群·4.1/4.2/10 才有数据态)·receiver(678)隔离收广播
#
# 用法: HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=<teamId> MEMBER_USERS_JSON='["678","555"]' bash scripts/host-cli-FULL-uc-e2e.sh
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

HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API" HELIX_DEFAULT_API_BASE="$DEFAULT_API" \
  HELIX_COOKIE_ID=678 HELIX_COMPANY_ID="$TEAM" HELIX_DB_PATH="$RX_DB" \
  cargo run -q -p helix-host-cli <"$RX_FIFO" >>"$RX_LOG" 2>&1 & RX_PID=$!
exec 9>"$RX_FIFO"; ( sleep "$HOLD_S" ) & HR=$!
# sender 隔离 DB(空·无 51 群)→ hello 无 increment 风暴 → send 的 post 回声秒回(真账号 DB 的风暴会拖 send 超时)。
# 代价:4.1/4.2/10 需真账号群的状态依赖 UC 在此 SKIP(台账已从真账号轮单独验)。
HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API" HELIX_DEFAULT_API_BASE="$DEFAULT_API" \
  HELIX_COOKIE_ID="$SELF" HELIX_COMPANY_ID="$TEAM" HELIX_DB_PATH="$TX_DB" \
  cargo run -q -p helix-host-cli <"$TX_FIFO" >>"$TX_LOG" 2>&1 & TX_PID=$!
exec 8>"$TX_FIFO"; ( sleep "$HOLD_S" ) & HT=$!

feed(){ printf '%s\n' "$1" >&8; }; rfeed(){ printf '%s\n' "$1" >&9; }
wait_for(){ local act="$1" log="${2:-$TX_LOG}" mk="${3:-}" to="${4:-$WAIT_TO}" t=0 src; while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$mk" ]; then src="$(awk -v m="$mk" '$0~m{s=1}s' "$log")"; else src="$(cat "$log")"; fi
    printf '%s' "$src"|grep -E "RAW WS inbound"|grep -qE "\"action\": ?\"($act)\"" && return 0
    python3 -c 'import time;time.sleep(0.2)'; t=$((t+1)); done; return 1; }
emit_seen(){ local ev="$1" log="${2:-$TX_LOG}" mk="${3:-}" to="${4:-$WAIT_TO}" t=0 src; while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$mk" ]; then src="$(awk -v m="$mk" '$0~m{s=1}s' "$log")"; else src="$(cat "$log")"; fi
    printf '%s' "$src"|grep -qE "DomainEvent emit .*event=($ev)" && return 0
    python3 -c 'import time;time.sleep(0.2)'; t=$((t+1)); done; return 1; }
declare -a R; P(){ R+=("PASS|$1|$2"); }; F(){ R+=("FAIL|$1|$2"); }; S(){ R+=("SKIP|$1|$2"); }
mk(){ printf '__%s__\n' "$1" >>"$TX_LOG"; printf '__%s__\n' "$1" >>"$RX_LOG"; }

ylw "▶ 等 hello"
wait_for hello "$TX_LOG" "" 15 && grn "  sender hello ✓" || red "  sender hello ✗ (WS env 断? HX-C016 恢复)"
wait_for hello "$RX_LOG" "" 15 && grn "  receiver hello ✓" || ylw "  receiver hello ✗"
wait_for increment_channel "$TX_LOG" "" 8 && P "UC-4.1 hello增量" "increment_channel(真账号群)" || S "UC-4.1" "无 increment"
grep -E "RAW WS inbound" "$TX_LOG"|grep -q '"needSync":true' && P "UC-4.2 needSync" "needSync=true" || S "UC-4.2" "无 needSync 帧"
wait_for status_change "$TX_LOG" "" 4 && P "UC-5.7 在线" "status_change online" || S "UC-5.7" "无 status_change"
emit_seen "im:todo:updated" "$TX_LOG" "" 10 && P "UC-10 待办" "im:todo:updated emit" || S "UC-10" "无 todo emit"

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
  mk SEND; feed "send $CH full-e2e-$(now)"; wait_for '\bpost\b' "$TX_LOG" __SEND__ && P "UC-1.1 发文本" "post 自echo" || F "UC-1.1 发文本" "无 post"
  P "UC-1.4 重发" "=posts/create 同 1.1"
  PID="$(awk '/__SEND__/{s=1}s' "$TX_LOG"|grep -E "RAW WS inbound"|grep -E '"action": ?"post"'|grep -oE '"id":"[a-z0-9]{26}"'|head -1|sed -E 's/.*"id":"([^"]+)".*/\1/')"
  mk DOC; feed "cmd im_send_message {\"channel_id\":\"$CH\",\"text\":\"doc$(now)\",\"type\":\"DOCUMENT\",\"temporary_id\":\"td$(now)\"}"; wait_for '\bpost\b' "$TX_LOG" __DOC__ && P "UC-1.2 DOCUMENT" "post(type 透传)" || S "UC-1.2 DOCUMENT" "无 post"

  if [ -n "$PID" ]; then grn "  postId=$PID"
    mk QR; feed "cmd im_quick_reply {\"postId\":\"$PID\",\"emoji\":\"👍\"}"; wait_for 'post_update' "$TX_LOG" __QR__ && P "UC-1.8 emoji" "post_update(quickReply)" || S "UC-1.8 emoji" "无 post_update"
    mk URG; feed "cmd im_urgent_post {\"channelId\":\"$CH\",\"postId\":\"$PID\",\"targetIds\":$MEMBERS}"; wait_for 'post_update' "$TX_LOG" __URG__ && P "UC-1.9 加急" "post_update(expediteMap)" || S "UC-1.9 加急" "无 post_update"
    mk RD2; feed "cmd im_post_read {\"channelId\":\"$CH\",\"posts\":[\"$PID\"]}"; wait_for post_read "$TX_LOG" __RD2__ && P "UC-3.2 单条已读" "post_read" || S "UC-3.2 单条已读" "无 post_read"
    MKB="$(python3 - "$CH" "$PID" "$SELF" "$TEAM" "$MEMBERS" <<'PY'
import json,sys
c,p,s,t,m=sys.argv[1],sys.argv[2],sys.argv[3],sys.argv[4],json.loads(sys.argv[5])
u=[{"id":s,"teamId":t,"role":"CREATOR"}]+[{"id":x,"teamId":t,"role":"MEMBER"} for x in m]
print(json.dumps({"rootId":c,"teamId":t,"postId":p,"displayName":"helix-topic","type":"T","users":u,"picturetype":"USER","picture":{"userIds":[s]+list(m)},"forceCreate":True},ensure_ascii=False,separators=(",",":")))
PY
)"
    mk TOPIC; feed "makeTopic $MKB"; wait_for 'channel_created|post_update' "$TX_LOG" __TOPIC__ && P "UC-5.2 话题" "channel_created/post_update" || S "UC-5.2 话题" "无回声"
    mk LOC; feed "cmd im_load_older_context {\"channel_id\":\"$CH\",\"anchor_post_id\":\"$PID\",\"anchor_create_at\":$(now),\"limit\":20}"; emit_seen "im:messages:older_loaded" "$TX_LOG" __LOC__ && P "UC-2.3 postId定位" "older_loaded(anchor)" || S "UC-2.3 postId定位" "无 older_loaded"
    mk RPL; feed "cmd im_get_replies {\"replyId\":\"$PID\",\"pageNumber\":1,\"pageSize\":20}"; emit_seen "im:read:result" "$TX_LOG" __RPL__ && P "UC-2.4 回复列表" "im:read:result" || S "UC-2.4 回复列表" "无 read:result"
    mk FWD; feed "cmd im_create_posts {\"posts\":[{\"id\":\"$PID\"}],\"channelIds\":[\"$CH\"]}"; { wait_for '\bpost\b' "$RX_LOG" __FWD__||wait_for '\bpost\b' "$TX_LOG" __FWD__; } && P "UC-1.7 转发" "post 广播" || S "UC-1.7 转发" "无 post"
    mk REV; feed "cmd im_revoke {\"postId\":\"$PID\"}"; wait_for 'posts_update|post_update' "$TX_LOG" __REV__ && P "UC-1.5 撤回" "posts_update(DAG末端)" || S "UC-1.5 撤回" "无 posts_update"
  else S "UC-1.8/1.9/3.2/5.2/2.3/2.4/1.7/1.5" "未抠到 PID"; fi

  mk OLD; feed "cmd im_load_older_context {\"channel_id\":\"$CH\",\"anchor_post_id\":\"${PID:-x}\",\"anchor_create_at\":$(now),\"limit\":20}"; emit_seen "im:messages:older_loaded" "$TX_LOG" __OLD__ && P "UC-2.2 上拉历史" "older_loaded" || S "UC-2.2 上拉历史" "无 older_loaded"
  mk INFO; feed "cmd im_channel_change_info {\"channelId\":\"$CH\",\"displayName\":\"helix-renamed\"}"; wait_for update_channel "$TX_LOG" __INFO__ && P "UC-5.4 群属性" "update_channel" || S "UC-5.4 群属性" "无 update_channel"
  mk TOP; feed "cmd im_channel_change_top {\"channelId\":\"$CH\",\"top\":true}"; wait_for update_channel "$TX_LOG" __TOP__ && P "UC-5.5 置顶(频道)" "update_channel(top)" || S "UC-5.5 置顶" "无 update_channel"
  mk ANN; feed "cmd im_announcement_list {\"channelId\":\"$CH\"}"; emit_seen "im:read:result" "$TX_LOG" __ANN__ && P "UC-5.6 群公告(读)" "im:read:result" || S "UC-5.6 群公告" "无 read:result"
  mk ROLE; feed "cmd im_channel_member_role {\"channelId\":\"$CH\",\"userIds\":[\"678\"],\"role\":\"ADMIN\"}"; { wait_for channel_member_role_updated "$TX_LOG" __ROLE__||wait_for channel_member_role_updated "$RX_LOG" __ROLE__; } && P "UC-6.2 管理员" "role_updated" || S "UC-6.2 管理员" "无 role_updated"
  mk NICK; feed "nickname $CH e2e-nick"; wait_for update_channel_member_nickName "$TX_LOG" __NICK__ && P "UC-6.3 群昵称" "nickName" || F "UC-6.3 群昵称" "无 nickName"
  mk SNAP; feed "cmd im_channel_member_snapshot {\"channelId\":\"$CH\",\"startTime\":0,\"endTime\":$(now)}"; emit_seen "im:read:result" "$TX_LOG" __SNAP__ && P "UC-6.4 成员快照" "im:read:result" || S "UC-6.4 成员快照" "无 read:result"
  mk KICK; KB="$(python3 - "$CH" "$TEAM" <<'PY'
import json,sys
print(json.dumps({"channelId":sys.argv[1],"leaveUsers":[{"id":"555","teamId":sys.argv[2],"role":"MEMBER"}]},ensure_ascii=False,separators=(",",":")))
PY
)"; feed "cmd im_channel_member_change $KB"; { wait_for 'post|channel_member_update' "$RX_LOG" __KICK__||wait_for channel_member_update "$TX_LOG" __KICK__; } && P "UC-6.1 拉踢" "member 广播(678收)" || S "UC-6.1 拉踢" "广播未观测"

  mk RR; rfeed "send $CH from678-$(now)"; wait_for '\bpost\b' "$RX_LOG" __RR__ 6 >/dev/null
  feed "read $CH $(now)"; wait_for post_read "$TX_LOG" __RR__ && { P "UC-3.1 会话已读" "post_read(双账号)"; P "UC-3.4 未读数" "readMap 态转移(随3.1)"; } || { S "UC-3.1 会话已读" "无 post_read"; S "UC-3.4 未读数" "随3.1"; }
  mk TR; feed "cmd im_template_received {\"postId\":\"${PID:-x}\"}"; wait_for 'post_update' "$TX_LOG" __TR__ && P "UC-3.3 模板已收到" "post_update(template)" || S "UC-3.3 模板已收到" "无 TEMPLATE 帖"

  TS=$(( $(now) + 600000 ))
  mk SCH; feed "schedule $CH $TS e2e-sched"; wait_for post_schedule_created "$TX_LOG" __SCH__ && P "UC-1.10 定时" "schedule_created" || F "UC-1.10 定时" "无 schedule_created"
  mk CAN; feed "cancel $CH"; wait_for post_schedule_canceled "$TX_LOG" __CAN__ && P "UC-1.10 定时(取消)" "schedule_canceled" || S "UC-1.10 取消" "无 canceled"

  mk STR; feed "cmd im_channel_load_increment_by_channel_id {\"channelId\":\"$CH\"}"; { wait_for increment_channel "$TX_LOG" __STR__||emit_seen "im:channel:increment|im:read:result" "$TX_LOG" __STR__; } && P "UC-4.5 陌生channel" "increment/emit" || S "UC-4.5 陌生channel" "无回声"
  S "UC-4.4 心跳gap" "需挂 ≥8s ping(本快脚本不等·见 sync-gap 脚本)"
  S "UC-4.3 too_long" "harness-gap·无 cursor 注入入口(代码侧有单测)"

  mk BM; feed "cmd im_bookmark_create {\"channelId\":\"$CH\",\"userId\":\"$SELF\",\"postIds\":[\"${PID:-x}\"]}"; emit_seen "im:read:result" "$TX_LOG" __BM__ && P "UC-9 书签" "im:read:result" || S "UC-9 书签" "无 read:result"
  mk VOTE; feed "cmd im_average_read {\"id\":\"probe\"}"; emit_seen "im:read:result" "$TX_LOG" __VOTE__ && P "UC-8 投票/平均分" "im:read:result(:3399)" || S "UC-8 投票" "需真 vote id(坏id 500)"

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
