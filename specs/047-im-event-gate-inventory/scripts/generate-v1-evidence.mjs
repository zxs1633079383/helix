import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const FEATURE_DIR = resolve(SCRIPT_DIR, '..');
const ROOT = resolve(FEATURE_DIR, '..', '..');
const MATRIX_PATH = join(ROOT, 'crates/helix-im/contracts/strict-matrix.v1.json');
const HELIX_TESTS = join(ROOT, 'crates/helix-im/tests');
const WDIO_SPECS =
  '/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs';
const OUTPUT_DIR = join(FEATURE_DIR, '.v1');
const STORAGE_SCHEMA_LOCK_PATH = join(FEATURE_DIR, 'contracts', 'storage-schema-lock.v1.json');
const STORAGE_SCHEMA_LOCK = JSON.parse(readFileSync(STORAGE_SCHEMA_LOCK_PATH, 'utf8'));

const STORAGE_PROFILES = {
  sync: {
    remote: [
      ['channel', ['id', 'display_name', 'type', 'last_post', 'last_post_at', 'delete_at', 'update_at']],
      ['channel_member', ['channel_id', 'user_id', 'role', 'unread_count', 'last_post', 'last_post_at', 'last_read_seq']],
      ['post', ['id', 'temporary_id', 'channel_id', 'user_id', 'type', 'message', 'props', 'revoke', 'create_at', 'update_at']],
      ['channel_event', ['channel_id', 'event_seq', 'event_type', 'msg_id', 'actor_id', 'payload', 'create_at']],
      ['channel_event_head', ['channel_id', 'committed_seq']],
    ],
    local: [
      ['channel', ['id', 'display_name', 'type', 'last_post', 'last_post_at', 'delete_at', 'last_event_seq']],
      ['channel_member', ['channel_id', 'user_id', 'role', 'unread_count', 'last_post', 'last_post_at', 'last_read_seq']],
      ['message', ['temporary_id', 'id', 'channel_id', 'user_id', 'type', 'message', 'props', 'event_seq', 'revoke', 'create_at', 'update_at']],
      ['channel_event_cursor', ['channel_id', 'last_event_seq', 'terminal_event_seq', 'updated_at']],
      ['channel_event_ledger', ['tenant_id', 'channel_id', 'event_seq', 'event_kind', 'message_id', 'coverage_id']],
      ['channel_sync_coverage', ['coverage_id', 'tenant_id', 'channel_id', 'from_seq', 'to_seq', 'event_count', 'commit_state']],
    ],
  },
  post: {
    remote: [
      ['post', ['id', 'temporary_id', 'channel_id', 'user_id', 'type', 'message', 'props', 'viewers', 'revoke', 'quick_reply', 'read_bits', 'expedite_map', 'create_at', 'update_at']],
      ['channel_member', ['channel_id', 'user_id', 'unread_count', 'last_post', 'last_post_at', 'mention_count', 'urgent_mention_count', 'last_read_seq']],
      ['channel_event', ['channel_id', 'event_seq', 'event_type', 'msg_id', 'actor_id', 'payload', 'create_at']],
      ['delivery_outbox', ['channel_id', 'event_seq', 'post_id', 'track_id', 'status', 'attempts']],
    ],
    local: [
      ['message', ['temporary_id', 'id', 'channel_id', 'user_id', 'type', 'message', 'props', 'viewers', 'quick_reply', 'read_bits', 'expedite_map', 'event_seq', 'send_status', 'revoke', 'create_at', 'update_at']],
      ['channel_member', ['channel_id', 'user_id', 'unread_count', 'last_post', 'last_post_at', 'mention_count', 'urgent_mention_count', 'last_read_seq']],
      ['channel_event_cursor', ['channel_id', 'last_event_seq', 'terminal_event_seq', 'updated_at']],
      ['channel_event_ledger', ['tenant_id', 'channel_id', 'event_seq', 'event_kind', 'message_id', 'coverage_id']],
    ],
  },
  schedule: {
    remote: [
      ['post_schedule', ['id', 'temporary_id', 'channel_id', 'user_id', 'type', 'message', 'props', 'status', 'scheduled_at', 'update_at']],
      ['channel_member', ['channel_id', 'user_id', 'has_schedule_post', 'schedule_post_at']],
    ],
    local: [
      ['channel_schedule', ['channel_id', 'schedule_id', 'owner_user_id', 'message', 'message_preview', 'send_at', 'status', 'revision']],
      ['channel', ['id', 'has_schedule_post', 'schedule_id', 'schedule_owner_user_id', 'schedule_message', 'schedule_send_at', 'schedule_status', 'schedule_revision']],
    ],
  },
  timeline: {
    remote: [
      ['post', ['id', 'temporary_id', 'channel_id', 'user_id', 'type', 'message', 'props', 'revoke', 'create_at', 'update_at']],
    ],
    local: [
      ['message', ['temporary_id', 'id', 'channel_id', 'user_id', 'type', 'message', 'props', 'event_seq', 'send_status', 'revoke', 'create_at', 'update_at']],
      ['channel_member', ['channel_id', 'user_id', 'unread_count', 'last_post', 'last_post_at', 'last_read_seq']],
      ['channel_event_cursor', ['channel_id', 'last_event_seq', 'terminal_event_seq', 'updated_at']],
    ],
  },
  read: {
    remote: [
      ['post', ['id', 'channel_id', 'user_id', 'read_bits', 'snapshot_id', 'update_at']],
      ['channel_member', ['channel_id', 'user_id', 'unread_count', 'mention_count', 'urgent_mention_count', 'last_read_seq', 'visible_read_count']],
      ['channel_event', ['channel_id', 'event_seq', 'event_type', 'msg_id', 'actor_id', 'payload', 'create_at']],
    ],
    local: [
      ['message', ['temporary_id', 'id', 'channel_id', 'user_id', 'read_bits', 'event_seq', 'update_at']],
      ['channel_member', ['channel_id', 'user_id', 'unread_count', 'mention_count', 'urgent_mention_count', 'last_read_seq']],
      ['channel_event_cursor', ['channel_id', 'last_event_seq', 'terminal_event_seq', 'updated_at']],
    ],
  },
  channel: {
    remote: [
      ['channel', ['id', 'display_name', 'type', 'header', 'purpose', 'owner', 'close', 'delete_at', 'create_at', 'update_at']],
      ['channel_member', ['id', 'channel_id', 'user_id', 'role', 'nick_name', 'delete_at', 'create_at', 'update_at']],
      ['channel_event', ['channel_id', 'event_seq', 'event_type', 'actor_id', 'payload', 'create_at']],
    ],
    local: [
      ['channel', ['id', 'display_name', 'type', 'header', 'purpose', 'owner', 'role', 'member_count', 'delete_at', 'created_at', 'updated_at']],
      ['channel_member', ['channel_id', 'user_id', 'role', 'nick_name', 'joined_at', 'updated_at']],
      ['channel_event_cursor', ['channel_id', 'last_event_seq', 'terminal_event_seq', 'updated_at']],
    ],
  },
  topic: {
    remote: [
      ['channel', ['id', 'display_name', 'type', 'root_id', 'parent_post_id', 'owner', 'create_at', 'update_at']],
      ['channel_topic', ['id', 'channel_id', 'post_id', 'user_id', 'last_post', 'total_msg_count', 'create_at', 'update_at']],
      ['channel_topic_members', ['id', 'channel_id', 'topic_id', 'user_id', 'last_post', 'total_msg_count', 'last_viewed_at']],
      ['post', ['id', 'temporary_id', 'channel_id', 'user_id', 'type', 'message', 'topic', 'create_at', 'update_at']],
    ],
    local: [
      ['channel', ['id', 'display_name', 'type', 'root_id', 'root_post_id', 'owner', 'member_count', 'created_at', 'updated_at']],
      ['channel_member', ['channel_id', 'user_id', 'role', 'joined_at', 'updated_at']],
      ['message', ['temporary_id', 'id', 'channel_id', 'user_id', 'type', 'message', 'topic', 'event_seq', 'send_status', 'create_at', 'update_at']],
    ],
  },
  settings: {
    remote: [
      ['channel', ['id', 'display_name', 'header', 'purpose', 'mention_permission', 'notice_permission', 'top_permission', 'change_meta', 'update_at']],
      ['channel_notice', ['id', 'channel_id', 'content', 'read_member', 'read_count']],
    ],
    local: [
      ['channel', ['id', 'display_name', 'header', 'purpose', 'mention_permission', 'notice_permission', 'top_permission', 'updated_at']],
    ],
  },
};

const GATE_STORAGE_PROFILE = {
  'G-S1': 'sync',
  'G-S2': 'sync',
  'G-S3': 'sync',
  'G-S4': 'sync',
  'G-01': 'post',
  'G-02': 'post',
  'G-03a': 'post',
  'G-04': 'post',
  'G-04a': 'post',
  'G-05': 'post',
  'G-06': 'post',
  'G-07': 'post',
  'G-08': 'schedule',
  'G-09': 'schedule',
  'G-10': 'timeline',
  'G-10a': 'timeline',
  'G-10b': 'timeline',
  'G-10c': 'timeline',
  'G-10d': 'timeline',
  'G-11': 'timeline',
  'G-11a': 'timeline',
  'G-12': 'timeline',
  'G-13': 'read',
  'G-14': 'read',
  'G-14a': 'post',
  'G-15a': 'channel',
  'G-15': 'topic',
  'G-16': 'channel',
  'G-16b': 'channel',
  'G-16c': 'settings',
};

const STORAGE_COLUMN_ALIASES = {
  channelId: 'channel_id',
  channelType: 'type',
  createAt: 'create_at',
  createdAt: 'created_at',
  deleteAt: 'delete_at',
  displayName: 'display_name',
  eventSeq: 'event_seq',
  lastPost: 'last_post',
  lastPostAt: 'last_post_at',
  lastReadSeq: 'last_read_seq',
  memberCount: 'member_count',
  mentionCount: 'mention_count',
  messagePreview: 'message_preview',
  nickName: 'nick_name',
  nickname: 'nick_name',
  quickReply: 'quick_reply',
  readBits: 'read_bits',
  revision: 'revision',
  scheduleId: 'schedule_id',
  sendAt: 'send_at',
  sendStatus: 'send_status',
  temporaryId: 'temporary_id',
  text: 'message',
  unreadCount: 'unread_count',
  updateAt: 'update_at',
  updatedAt: 'updated_at',
  urgentCount: 'urgent_mention_count',
  userId: 'user_id',
};

/** 校验所有 Gate 引用的表与列都来自本轮只读抓取的 schema lock。 */
function validateStorageProfiles() {
  for (const [profileName, profile] of Object.entries(STORAGE_PROFILES)) {
    for (const side of ['remote', 'local']) {
      const lockedTables =
        side === 'remote' ? STORAGE_SCHEMA_LOCK.remote_tables : STORAGE_SCHEMA_LOCK.local_tables;
      for (const [table, columns] of profile[side]) {
        if (!lockedTables[table]) {
          throw new Error(`${profileName}.${side} references unknown table: ${table}`);
        }
        for (const column of columns) {
          if (!lockedTables[table].includes(column)) {
            throw new Error(
              `${profileName}.${side}.${table} references unknown column: ${column}`,
            );
          }
        }
      }
    }
  }
}

/** 给 schema-verified 列生成不含真实业务数据的结构样例值。 */
function storageColumnSample(table, column, gateId, perspective) {
  const ids = SAMPLE_IDS;
  const eventSeq = gateId === 'G-04' || gateId === 'G-04a' ? 102 : 101;
  const rowUserId =
    table === 'channel_member' || table === 'channel_topic_members'
      ? perspective === 'receiver'
        ? ids.observer
        : ids.actor
      : ids.actor;
  const values = {
    id: table.includes('channel') ? ids.channel : ids.message,
    temporary_id: ids.temporary,
    channel_id: ids.channel,
    user_id: rowUserId,
    actor_id: ids.actor,
    msg_id: ids.message,
    post_id: ids.message,
    message_id: ids.message,
    type: table === 'channel' ? 'O' : 'TEXT',
    message: 'Gate review message',
    display_name: 'Gate Review Group',
    role: perspective === 'receiver' ? 'MEMBER' : 'ADMIN',
    nick_name: perspective === 'receiver' ? 'Observer' : 'Actor',
    event_seq: eventSeq,
    last_event_seq: eventSeq,
    terminal_event_seq: eventSeq,
    committed_seq: eventSeq,
    last_read_seq: perspective === 'receiver' ? 100 : 101,
    unread_count: perspective === 'receiver' ? 1 : 0,
    mention_count: 0,
    urgent_mention_count: 0,
    member_count: 2,
    msg_count: 1,
    total_msg_count: 1,
    event_count: 1,
    from_seq: 101,
    to_seq: 101,
    create_at: 1_700_000_000_000,
    created_at: 1_700_000_000_000,
    update_at: 1_700_000_000_100,
    updated_at: 1_700_000_000_100,
    last_post_at: 1_700_000_000_000,
    last_root_post_at: 1_700_000_000_000,
    joined_at: 1_700_000_000_000,
    scheduled_at: 1_700_000_060_000,
    send_at: 1_700_000_060_000,
    delete_at: 0,
    revoke: gateId === 'G-04' || gateId === 'G-04a' ? 1 : 0,
    close: gateId === 'G-16',
    status:
      table === 'delivery_outbox'
        ? 'delivered'
        : gateId === 'G-09'
          ? 'canceled'
          : 'scheduled',
    send_status: 'sent',
    revision: 8,
    schedule_id: 'schedule-review-001',
    owner_user_id: ids.actor,
    event_type: gateId === 'G-04' || gateId === 'G-04a' ? 3 : 1,
    event_kind: gateId === 'G-04' || gateId === 'G-04a' ? 'delete' : 'post',
    commit_state: 'committed',
    coverage_id: 'coverage-review-001',
    tenant_id: perspective === 'receiver' ? '999' : '444',
  };
  if (column === 'last_post') {
    return {
      id: ids.message,
      temporaryId: ids.temporary,
      channelId: ids.channel,
      userId: ids.actor,
      type: 'TEXT',
      message: 'Gate review message',
      revoke: gateId === 'G-04' || gateId === 'G-04a',
      createAt: 1_700_000_000_000,
      updateAt: 1_700_000_000_100,
    };
  }
  if (['props', 'payload', 'owner', 'change_meta'].includes(column)) {
    return column === 'payload' ? { state: 'committed' } : {};
  }
  if (['viewers'].includes(column)) {
    return ['all'];
  }
  if (['quick_reply', 'read_bits', 'expedite_map'].includes(column)) {
    return column === 'read_bits' ? '0' : [];
  }
  return values[column] ?? `<${table}.${column}>`;
}

/** 将 Gate 的远端或本地持久层依据展开成可逐列复核的行结构。 */
function storageBasis(gateId, side, perspective = 'sender') {
  const profileName = GATE_STORAGE_PROFILE[gateId];
  const profile = STORAGE_PROFILES[profileName];
  const tables = profile[side].map(([table, columns]) => ({
    table,
    columns,
    row_sample: Object.fromEntries(
      columns.map((column) => [
        column,
        storageColumnSample(table, column, gateId, perspective),
      ]),
    ),
  }));
  if (side === 'remote') {
    return {
      authority: 'REMOTE_POSTGRESQL_PERSISTED_FACT',
      profile: profileName,
      endpoint: `${STORAGE_SCHEMA_LOCK.evidence.remote_postgresql.server_addr}:${STORAGE_SCHEMA_LOCK.evidence.remote_postgresql.server_port}`,
      database: STORAGE_SCHEMA_LOCK.evidence.remote_postgresql.database,
      schema: STORAGE_SCHEMA_LOCK.evidence.remote_postgresql.schema,
      tables,
    };
  }
  const tenant = perspective === 'receiver' ? '999' : '444';
  return {
    authority: 'LOCAL_SQLITE_AFTER_PERSIST_OK',
    profile: profileName,
    tenant,
    database: join(
      STORAGE_SCHEMA_LOCK.evidence.local_sqlite.storage_root,
      STORAGE_SCHEMA_LOCK.evidence.local_sqlite.tenant_databases[tenant],
    ),
    tables,
  };
}

/** 把出站 data 顶层字段映射到本地列；未映射项明确留给人工判定而不冒充持久字段。 */
function outboundFieldLineage(gateId, event, data) {
  const profile = STORAGE_PROFILES[GATE_STORAGE_PROFILE[gateId]];
  const available = profile.local.flatMap(([table, columns]) =>
    columns.map((column) => ({ table, column })),
  );
  const direct = {};
  const nonStorage = [];
  for (const field of Object.keys(data)) {
    const column = STORAGE_COLUMN_ALIASES[field] ?? field;
    const candidateColumns =
      field === 'channelId'
        ? [column, 'id']
        : field === 'closed'
          ? [column, 'close', 'delete_at']
          : [column];
    const matches = available
      .filter((candidate) => candidateColumns.includes(candidate.column))
      .map((candidate) => `${candidate.table}.${candidate.column}`);
    if (matches.length > 0) {
      direct[field] = matches;
      continue;
    }
    if (field === 'messages' || field === 'posts') {
      direct[field] = ['message.*'];
      continue;
    }
    if (field === 'items') {
      direct[field] = ['channel.*'];
      continue;
    }
    if (field === 'members' || field === 'leaves') {
      direct[field] = ['channel_member.*'];
      continue;
    }
    if (field === 'settings') {
      direct[field] = ['channel.display_name/header/purpose/mention_permission/notice_permission/top_permission'];
      continue;
    }
    nonStorage.push(field);
  }
  return {
    event,
    direct_local_sources: direct,
    non_storage_or_derived_fields: nonStorage,
    rule:
      'direct_local_sources 可由本地行读回；non_storage_or_derived_fields 是 command correlation、分页元数据、角色派生或当前待确认字段，不能宣称来自 SQLite。',
  };
}

const GATE_GROUPS = {
  'sync/recovery': ['G-S1', 'G-S2', 'G-S3', 'G-S4'],
  post: [
    'G-01',
    'G-02',
    'G-03a',
    'G-04',
    'G-04a',
    'G-05',
    'G-06',
    'G-07',
    'G-14',
    'G-14a',
  ],
  schedule: ['G-08', 'G-09'],
  'timeline/read': [
    'G-10',
    'G-10a',
    'G-10b',
    'G-10c',
    'G-10d',
    'G-11',
    'G-11a',
    'G-12',
    'G-13',
  ],
  'channel/channel-member': ['G-15a', 'G-15', 'G-16', 'G-16b', 'G-16c'],
};

const GATE_IDS = Object.values(GATE_GROUPS).flat();

const EVENT_SOURCES = {
  'im:post:sending': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:post:received': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:updated': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:deleted': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:read': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:channel:read_echo': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:send-failed': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:posts:batch-result': 'crates/helix-im/src/query/render_ready/batch_result.rs',
  'im:post:batch-updated': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:sync:state': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:sync:too_long': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:channels:loaded': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:channel:increment': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:channel:update': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:operation:status': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:messages:query_result': 'crates/helix-im/src/query/core.rs',
  'im:read:result': 'crates/helix-im/src/query/read_relay.rs',
  'im:channel:replies': 'crates/helix-im/src/query/render_ready/replies.rs',
  'im:channel:members': 'crates/helix-im/src/query/render_ready/members.rs',
  'im:post:readers': 'crates/helix-im/src/query/render_ready/receipts.rs',
  'im:channel:created': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:closed': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:left': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:member-updated': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:settings-updated': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:schedule-created': 'crates/helix-im/src/acl/to_effect_s1/schedule.rs',
  'im:channel:schedule-canceled': 'crates/helix-im/src/acl/to_effect_s1/schedule.rs',
};

const POST_CHANNEL_UPDATE_TARGET = {
  event: 'im:channel:update',
  source: 'crates/helix-im/src/dialog_patch.rs',
  source_status: 'TARGET_REQUIRED',
};

const POST_REVOKE_TARGET = {
  event: 'im:post:revoke',
  source: 'crates/helix-im/src/acl/projection_message_effects.rs',
  source_status: 'TARGET_REQUIRED',
};

const POST_QUICK_REPLY_TARGET = {
  event: 'im:post:updated',
  source: 'crates/helix-im/src/acl/projection_message_effects.rs',
  source_status: 'TARGET_REQUIRED',
};

const WINDOW_RESULT_TARGET = {
  event: 'im:messages:query_result',
  source: 'crates/helix-im/src/query/core.rs',
  source_status: 'TARGET_REQUIRED',
};

const LEAVER_LEFT_TARGET = {
  event: 'im:channel:left',
  source: 'crates/helix-im/src/acl/to_effect_s1.rs',
  source_status: 'TARGET_REQUIRED',
};

const GATE_EMITS = {
  'G-S1': {
    current: ['im:channel:increment', 'im:channels:loaded'],
    required: ['每个可见频道的增量事实', '全批持久提交后的 loaded 终态'],
  },
  'G-S2': {
    current: [
      'im:post:received',
      'im:post:updated',
      'im:post:deleted',
      'im:post:batch-updated',
      'im:channel:update',
      'im:sync:state',
    ],
    required: ['离线 mutation 的逐事实 emit', 'cursor 与全批提交后的 sync state'],
  },
  'G-S3': {
    current: ['im:sync:state', 'im:sync:too_long'],
    required: ['gap 起止状态', '补偿失败或 too-long 状态'],
  },
  'G-S4': {
    current: ['im:channel:increment', 'im:channel:update', 'im:post:received'],
    required: ['陌生频道补全事实', '触发补全的原消息事实'],
  },
  'G-01': {
    current: ['im:post:sending', 'im:post:received', 'im:post:send-failed'],
    target: [POST_CHANNEL_UPDATE_TARGET],
    required: [
      'sender optimistic',
      'sender terminal',
      'sender channel lastPost + absolute unreadCount=0',
      'receiver authority',
      'receiver channel lastPost + absolute unreadCount increment',
    ],
    inferred: [
      '当前普通 post 写后结果仍进入旧 Dialog Frame 路径；独立 im:channel:update 仅有 channel_id，尚未直接 Emit 当前成员读回的 lastPost 与绝对 unreadCount。',
    ],
  },
  'G-02': {
    current: ['im:post:sending', 'im:post:received', 'im:post:send-failed'],
    target: [POST_CHANNEL_UPDATE_TARGET],
    required: ['文档 optimistic/terminal', 'receiver 文档 authority', '双方频道摘要绝对态'],
    inferred: ['文档消息同样需要写后 im:channel:update；当前 post 路径没有该 raw 绝对态事件。'],
  },
  'G-03a': {
    current: ['im:post:send-failed', 'im:post:sending', 'im:post:received'],
    target: [POST_CHANNEL_UPDATE_TARGET],
    required: [
      'failed 行触发 im_retry_send',
      '同 temporaryId 重新进入 im:post:sending',
      'received 或 send-failed 权威终态',
      '成功后的双方频道摘要绝对态',
    ],
    inferred: ['重试成功复用普通 post 写后链路，但当前没有携 lastPost/unreadCount 的 raw channel 事件。'],
  },
  'G-04': {
    current: [],
    target: [POST_REVOKE_TARGET, POST_CHANNEL_UPDATE_TARGET],
    required: [
      '同 postId 的 revoke=true 幂等终态',
      'sender/receiver 都从本地 message 行读回同一撤回事实',
      '撤回最后一条后分别读回各自 channel_member.last_post/unread_count',
      '展示文案只由 system_event.actorId + viewer identity 派生，不进入 raw 持久 Emit',
    ],
    inferred: [
      '当前 Go 无条件广播 posts_update，但仅在撤回结果等于 GetPostLaster() 返回的全表最新 post 时广播 update_channel；该判断不是 channel-scoped，且副链失败会降级为只有 posts_update。',
      '当前 Go update_channel.data.lastPost 是 JSON 字符串；当前 Helix 撤回主链落 revoke=1 后 emit im:post:deleted + im:post:batch-updated，未 emit im:post:revoke。',
      '当前 Go 只广播鉴权 actorId，不广播 actorName；消费端若需要具体名称，必须查询成员/用户目录，不能把 actorName 冒充 message 行字段。',
      '当前 Helix im:channel:update 仅携 channel_id，且 lastPost JSON 字符串没有被解析为可用对象；WDIO G04 也明确把 dialog mutation 标为 N/A。',
    ],
  },
  'G-04a': {
    current: ['im:post:deleted', 'im:post:batch-updated', 'im:channel:update'],
    required: ['离线撤回重放', '恢复后的频道摘要事实'],
  },
  'G-05': {
    current: ['im:posts:batch-result', 'im:post:received'],
    target: [POST_CHANNEL_UPDATE_TARGET],
    required: ['每个目标的批结果', '各目标频道 receiver authority', '目标频道摘要绝对态'],
    inferred: ['目标 post 可更新 channel 行，但当前 raw 出站未携目标频道 lastPost/unreadCount 绝对态。'],
  },
  'G-06': {
    current: [],
    target: [POST_QUICK_REPLY_TARGET],
    required: [
      '同 postId 的 quickReply 绝对态',
      'sender/receiver 直接消费同一 [{emoji,userIds}] 结构',
    ],
    inferred: [
      '远端 post.quick_reply 是 {emoji:[userId]} JSONB；Go 在 WS 边界一次性规范化为 quickReply:[{emoji,userIds}]。目标 Helix 将该数组原样写入 message.quick_reply(TEXT JSON)并原样 raw emit，不再塞进 props.quickReply 或派生 reactions/reactionCount。当前 edit_content_op 只写 props，message.quick_reply 没有更新。',
    ],
  },
  'G-07': {
    current: ['im:post:updated', 'im:post:readers', 'im:channel:update'],
    required: ['加急状态', '确认成员集合', 'viewer-local 频道计数'],
  },
  'G-08': {
    current: ['im:channel:schedule-created', 'im:operation:status'],
    required: ['创建者排程事实', '参与者在真正发送前不得收到 post'],
  },
  'G-09': {
    current: ['im:channel:schedule-canceled', 'im:operation:status'],
    required: ['取消者排程终态', '参与者不得收到已取消 post'],
  },
  'G-10': {
    current: ['im:messages:query_result'],
    required: ['latest window、bounds、revision 与 continuation token'],
    inferred: ['现有 query_result 需要补齐窗口元数据，不能依赖 UI 层另算'],
  },
  'G-10a': {
    current: ['im:messages:query_result'],
    required: ['older page、头部边界与 nextOlder token'],
    inferred: ['当前源码未证明 older 专用终态字段完整'],
  },
  'G-10b': {
    current: ['im:messages:query_result', 'im:post:received'],
    required: ['newer page、尾部边界与 nextNewer token'],
    inferred: ['当前源码未证明 newer 专用终态字段完整'],
  },
  'G-10c': {
    current: ['im:post:received'],
    target: [POST_CHANNEL_UPDATE_TARGET, WINDOW_RESULT_TARGET],
    required: ['append 后窗口淘汰结果、bounds 与 revision'],
    inferred: ['缺少表达头部淘汰后的 raw emit 合同'],
  },
  'G-10d': {
    current: ['im:post:received'],
    target: [POST_CHANNEL_UPDATE_TARGET, WINDOW_RESULT_TARGET],
    required: ['离底 anchor 保持与 new-message indicator'],
    inferred: ['缺少表达 anchor/提示状态的 raw emit 合同'],
  },
  'G-11': {
    current: ['im:read:result', 'im:messages:query_result'],
    required: ['目标 post authority 与仅 actor 生效的定位元数据'],
    inferred: ['当前源码未证明 locate directive 的 raw emit 等价物'],
  },
  'G-11a': {
    current: ['im:messages:query_result', 'im:post:received'],
    required: ['未缓存目标、双向 token、anchor 与 stale-navigation rejection'],
    inferred: ['当前源码未证明定位后双向分页元数据完整'],
  },
  'G-12': {
    current: ['im:read:result', 'im:channel:replies'],
    required: ['thread snapshot、reply ids、revision 与请求关联键'],
  },
  'G-13': {
    current: ['im:channel:read_echo', 'im:channel:update'],
    required: ['actor viewer-local 已读绝对态', 'observer 计数不变'],
  },
  'G-14': {
    current: ['im:post:read', 'im:post:readers'],
    required: ['单 post 已读终态', 'sender/reader 各自可见 receipt'],
  },
  'G-14a': {
    current: ['im:post:updated'],
    required: ['templateReceived、confirmed count 与 reader ids'],
  },
  'G-15a': {
    current: ['im:channel:created', 'im:channel:members'],
    required: ['creator channel authority', '每个 member 的 roster authority'],
  },
  'G-15': {
    current: ['im:channel:created', 'im:post:received', 'im:post:updated'],
    required: ['topic channel authority', 'root post 与参与者可见性'],
  },
  'G-16': {
    current: ['im:channel:closed', 'im:channel:left'],
    required: ['close 与 self-left 两种不可混淆终态', 'observer 对应广播事实'],
  },
  'G-16b': {
    current: ['im:channel:member-updated', 'im:channel:members'],
    target: [LEAVER_LEFT_TARGET],
    required: ['remaining member 的 roster authority', 'leaver 的 viewer-local left 终态'],
    inferred: ['当前 Gate 清单未证明退出成员会收到独立 im:channel:left；该视角先作为待实现目标。'],
  },
  'G-16c': {
    current: ['im:channel:settings-updated', 'im:channel:update'],
    required: ['actor 保存终态', 'observer 最新 settingVersion 与字段绝对态'],
  },
};

const GATE_PRELUDE_EVENTS = {
  'G-01': ['im:post:sending'],
  'G-02': ['im:post:sending'],
  'G-03a': ['im:post:sending'],
};

const GATE_FAILURE_EVENTS = {
  'G-S3': ['im:sync:too_long'],
  'G-01': ['im:post:send-failed'],
  'G-02': ['im:post:send-failed'],
  'G-03a': ['im:post:send-failed'],
};

const GATE_ASSOCIATED_EVENTS = {
  'G-S2': ['im:channel:update'],
  'G-S4': ['im:channel:update'],
  'G-01': ['im:channel:update'],
  'G-02': ['im:channel:update'],
  'G-03a': ['im:channel:update'],
  'G-04': ['im:channel:update'],
  'G-04a': ['im:post:batch-updated', 'im:channel:update'],
  'G-05': ['im:channel:update'],
  'G-07': ['im:post:readers', 'im:channel:update'],
  'G-10c': ['im:channel:update'],
  'G-10d': ['im:channel:update'],
  'G-13': ['im:channel:update'],
  'G-14': ['im:post:readers'],
  'G-15a': ['im:channel:members'],
  'G-16b': ['im:channel:members'],
  'G-16c': ['im:channel:update'],
};

const MESSAGE_V3_SPLIT_GAPS = {
  'G-S1': [
    'increment_channel 的 program_full 仍把 unread/mention/urgent 个人状态写入 channel；成员批量 upsert 只保存 roster，尚未把当前登录用户的个人投影写入 channel_member。',
  ],
  'G-S2': [
    '离线 mutation/sync_dialog_patch 仍通过 bump_channel_unread_op 更新 channel，too-long 恢复也会重置 channel 的个人字段；分表后必须改为当前 auth_user_id 的 channel_member 原子写入与读回。',
    '离线 post 恢复后的 im:channel:update 仍是瘦信号，尚未携带 channel_member 写后读回的 lastPost/unreadCount 绝对态。',
  ],
  'G-S3': [
    'pong gap 的补偿链最终复用 S2；当前 too-long reset_channel_dialog_op 仍写分表后已从 channel 删除的 unread/mention/urgent 字段。',
  ],
  'G-S4': [
    '陌生频道补全同时复用 increment_channel 与普通 post：前者尚未把当前成员个人投影写入 channel_member，后者仍把未读累计写入 channel。',
  ],
  'G-01': [
    '普通 WS post 当前仍更新并读回 channel；分表后应更新 channel 公共 lastPost，并按 (channel_id,auth_user_id) 更新/读回 channel_member 个人摘要。',
    '发送者当前只是 unread_delta=0 而不是 channel_member.unread_count=0；接收者也尚未在 channel_member 上幂等 +1。',
  ],
  'G-02': [
    '文档消息复用普通 post 落库链，发送者绝对清零、接收者幂等 +1 与个人 lastPost 尚未迁到 channel_member。',
  ],
  'G-03a': [
    '重试成功复用普通 post 落库链，成功终态后的 channel_member 发送者清零/接收者加一尚未实现。',
  ],
  'G-04': [
    '撤回最后一条消息时，当前实现尚未以 (channel_id,viewer_user_id) 更新并读回双方各自的 channel_member.last_post/unread_count。',
  ],
  'G-04a': [
    '离线撤回恢复复用 sync 投影链；当前仍会重置或累计 channel 个人字段，尚未把恢复后的个人摘要归一到 channel_member。',
  ],
  'G-05': [
    '转发成功后的每个目标 post 仍复用普通 post 的 channel 未读写法；目标频道当前成员的 channel_member 摘要与未读尚未原子更新/读回。',
  ],
  'G-07': [
    'update_channel 已有 channel_member 绝对态 upsert，但同一处理仍通过 collect_present 把 unread/mention/urgent 重复写入 channel；目标 schema 删除这些列后该相关 Persist 会失败。',
    'channel_member 加急列仍写 urgent_mention_count/urgent_mention_user，尚未改为 MessageV3 的 urgent_count/urgent_current_name；im:channel:update 也仍是瘦信号。',
  ],
  'G-10b': [
    'newer 分页期间收到的新 post 复用普通 post 投影链，关联的 channel_member 未读与个人 lastPost 尚未按分表合同更新。',
  ],
  'G-10c': [
    '满窗口 append 的新 post 仍复用普通 post 投影链，关联的 channel_member 未读与个人 lastPost 尚未按分表合同更新。',
  ],
  'G-10d': [
    '离底 anchor 状态下的新 post 仍复用普通 post 投影链，关联的 channel_member 未读与个人 lastPost 尚未按分表合同更新。',
  ],
  'G-11a': [
    '未缓存定位期间乱序到达的新 post 仍复用普通 post 投影链，关联的 channel_member 未读与个人 lastPost 尚未按分表合同更新。',
  ],
  'G-13': [
    'update_channel 的已读绝对态虽已解析到 channel_member，但 collect_present 仍同步写 channel.unread_count/unread_post_id/mention/urgent；分表后必须只写当前成员行并从该行读回完整 im:channel:update。',
  ],
  'G-15a': [
    '创建群后的 channel_created/channel_member_update 仍调用 program_full 写入 channel 个人字段；roster upsert 尚未为创建者建立完整的 channel_member 个人会话投影。',
  ],
  'G-15': [
    '创建话题复用 channel_created 全量写入，仍会写分表后已删除的 channel 个人字段；topic 创建者的个人会话投影尚未归入 channel_member。',
  ],
  'G-16b': [
    'channel_member_update 仍复用 program_full 写入 channel 个人字段；分表后成员广播只能更新 channel 公共字段与独立 roster/member 行。',
  ],
  'G-16c': [
    '群属性 update_channel 已有 member upsert，但 collect_present 仍可能把个人 unread/mention/urgent 字段写回 channel，且 member 加急列仍是旧命名；目标 schema 下需要拆成公共 PATCH 与成员 PATCH。',
  ],
};

const SAMPLE_IDS = {
  channel: 'chreview000000000000000001',
  topic: 'chtopic0000000000000000001',
  message: 'msgreview00000000000000001',
  message2: 'msgreview00000000000000002',
  temporary: 'tmpreview00000000000000001',
  actor: 'user-actor-444',
  observer: 'user-observer-678',
  req: 'req-review-001',
};

const GATE_WS_ACTIONS = {
  'G-S1': ['hello', 'increment_channel', 'increment_channel_end'],
  'G-S2': [],
  'G-S3': ['pong'],
  'G-S4': ['increment_channel'],
  'G-01': ['post'],
  'G-02': ['post'],
  'G-03a': ['post'],
  'G-04': ['posts_update', 'revoke_update_channel'],
  'G-04a': ['posts_update'],
  'G-05': ['post'],
  'G-06': ['post_update'],
  'G-07': ['post_update', 'update_channel'],
  'G-08': ['post_schedule_created'],
  'G-09': ['post_schedule_canceled'],
  'G-10': [],
  'G-10a': [],
  'G-10b': ['post'],
  'G-10c': ['post'],
  'G-10d': ['post'],
  'G-11': [],
  'G-11a': ['post'],
  'G-12': [],
  'G-13': ['update_channel'],
  'G-14': ['post_read'],
  'G-14a': ['post_update'],
  'G-15a': ['channel_created'],
  'G-15': ['channel_created', 'post_update'],
  'G-16': ['channel_close'],
  'G-16b': ['channel_member_update'],
  'G-16c': ['update_channel', 'update_channel_notice'],
};

const COMMAND_SAMPLE_SOURCES = {
  im_send_message: 'crates/helix-im/tests/g01_shared/scenario.rs',
  im_retry_send: 'crates/helix-im/tests/g03_media_retry_contract_test.rs',
  im_relay_messages: 'crates/helix-im/tests/support/g05_scenario.rs',
  im_create_schedule: 'crates/helix-im/tests/outbound_commands.rs',
  im_post_read: 'crates/helix-im/tests/support/g14_scenario.rs',
  im_create_channel: 'crates/helix-im/tests/p3b_channel_outbound_test.rs',
  im_make_topic: 'crates/helix-im/tests/support/g15_scenario.rs',
};

/** 构造与当前命令 parser 字段一致的代表性 payload。 */
function commandSamplePayload(name, gateId) {
  const ids = SAMPLE_IDS;
  const samples = {
    im_query_dialog_list: { limit: 50 },
    im_sync_channels: {
      reason: gateId === 'G-S3' ? 'pong_gap' : 'offline_notify',
      channel_id: ids.channel,
      event_seq: 100,
    },
    im_channel_load_increment_by_channel_id: {
      channel_id: ids.channel,
      req_id: ids.req,
      is_read: false,
    },
    im_send_message:
      gateId === 'G-02'
        ? {
            channel_id: ids.channel,
            type: 'DOCUMENT',
            text: '【文档】 http://doc-pre.cses7.com/doc/review',
            props: {
              document: {
                version: 1,
                url: 'http://doc-pre.cses7.com/doc/review',
                title: '打开文档',
                placeholder: true,
              },
            },
            req_id: ids.req,
          }
        : {
            channel_id: ids.channel,
            text: 'Gate review message',
            type: 'TEXT',
            props: {},
            req_id: ids.req,
          },
    im_retry_send: { message_id: ids.temporary },
    im_revoke: { post_id: ids.message },
    im_relay_messages: {
      post_ids: [ids.message],
      target_channel_ids: [ids.topic],
      mode: 'individual',
      req_id: ids.req,
    },
    im_quick_reply: { post_id: ids.message, emoji: ':thumbsup:', req_id: ids.req },
    im_urgent_post: { post_id: ids.message, target_ids: [ids.observer] },
    im_urgent_confirm: { post_id: ids.message },
    im_create_schedule: {
      channel_id: ids.channel,
      message: '下午三点发送',
      schedule_post_at: 1_700_000_060_000,
      type: 'TEXT',
      props: {},
    },
    im_cancel_schedule: { channel_id: ids.channel },
    im_query_messages_by_channel: { channel_id: ids.channel, limit: 60 },
    im_load_older_context: {
      channel_id: ids.channel,
      anchor_post_id: ids.message,
      before: 1_700_000_000_000,
    },
    im_load_newer_context: {
      channel_id: ids.channel,
      anchor_post_id: ids.message,
      page_size: 60,
    },
    im_locate_message: {
      channel_id: ids.channel,
      message_id: ids.message,
      page_size: 60,
    },
    im_get_posts: { post_ids: [ids.message], req_id: ids.req, locate: true },
    im_get_replies: {
      reply_id: ids.message,
      req_id: ids.req,
      channel_id: ids.channel,
      root_message_id: ids.message,
      page_number: 1,
      page_size: 20,
    },
    im_get_reply_branch: {
      reply_first_level_id: ids.message,
      req_id: ids.req,
      channel_id: ids.channel,
      root_message_id: ids.message,
      page_number: 1,
      page_size: 20,
    },
    im_channels_view: { channels: [{ id: ids.channel, isRoot: true }] },
    im_post_read: { channel_id: ids.channel, posts: [ids.message] },
    im_template_received: { post_id: ids.message },
    im_create_channel: {
      type: 'O',
      user_ids: [ids.actor, ids.observer],
      display_name: 'Gate Review Group',
      force_create: true,
    },
    im_make_topic: {
      root_id: ids.message,
      user_ids: [ids.observer],
      display_name: 'Gate Review Topic',
      req_id: ids.req,
    },
    im_channel_close: { channel_id: ids.channel },
    im_channel_leave: { channel_id: ids.channel },
    im_channel_change_display_name: {
      channel_id: ids.channel,
      display_name: 'Gate Review Renamed',
    },
    im_channel_change_notice: { channel_id: ids.channel, notice: 'Review notice' },
    im_channel_change_info: {
      channel_id: ids.channel,
      purpose: 'Review purpose',
      header: 'Review header',
      expected_version: 7,
    },
  };
  const payload = samples[name];
  if (!payload) {
    throw new Error(`Command sample missing: ${name}`);
  }
  return payload;
}

/** 构造 G-05 逐条转发与合并转发两套 canonical command。 */
function g05CommandSamples() {
  const ids = SAMPLE_IDS;
  return [
    {
      sample_id: 'command-individual',
      payload: {
        post_ids: [ids.message, ids.message2],
        target_channel_ids: [ids.topic],
        mode: 'individual',
        req_id: 'req-review-individual-001',
      },
    },
    {
      sample_id: 'command-merged',
      payload: {
        post_ids: [ids.message, ids.message2],
        target_channel_ids: [ids.topic],
        mode: 'merged',
        req_id: 'req-review-merged-001',
      },
    },
  ];
}

/** 构造合并转发 durable props 中的两条来源消息快照。 */
function g05MergedItems() {
  const ids = SAMPLE_IDS;
  return [
    {
      id: ids.message,
      temporaryId: 'source-temporary-001',
      userId: ids.actor,
      userSnapshot: { userName: 'Actor' },
      type: 'TEXT',
      message: 'Forwarded message A',
      simpleMessage: 'Forwarded message A',
      sendStatus: 'sended',
      viewers: ['all'],
      props: {},
      createAt: 1_700_000_000_000,
    },
    {
      id: ids.message2,
      temporaryId: 'source-temporary-002',
      userId: ids.observer,
      userSnapshot: { userName: 'Observer' },
      type: 'TEXT',
      message: 'Forwarded message B',
      simpleMessage: 'Forwarded message B',
      sendStatus: 'sended',
      viewers: ['all'],
      props: {},
      createAt: 1_700_000_000_100,
    },
  ];
}

/** 构造 G-05 每种模式由服务端广播的目标频道 WS post authority。 */
function g05AuthoritySamples() {
  const ids = SAMPLE_IDS;
  const base = {
    v: 1,
    action: 'post',
    broadcast: {},
  };
  const individualPosts = [
    {
      id: 'forwarded-individual-001',
      temporaryId: 'relay-req-review-individual-001-0',
      seq: 201,
      createAt: 1_700_000_001_000,
      message: 'Forwarded message A',
    },
    {
      id: 'forwarded-individual-002',
      temporaryId: 'relay-req-review-individual-001-1',
      seq: 202,
      createAt: 1_700_000_001_001,
      message: 'Forwarded message B',
    },
  ].map((post, index) => ({
    sample_id: `ws-authority-individual-${index + 1}`,
    value: {
      ...base,
      tracing: { csesTrackId: 'req-review-individual-001' },
      data: {
        id: post.id,
        temporaryId: post.temporaryId,
        channelId: ids.topic,
        event_seq: post.seq,
        createAt: post.createAt,
        message: post.message,
        type: 'TEXT',
        userId: ids.actor,
        viewers: ['all'],
        props: { channel_event_seq: post.seq },
      },
      seq: post.seq,
    },
  }));
  const mergedItems = g05MergedItems();
  return [
    ...individualPosts,
    {
      sample_id: 'ws-authority-merged-1',
      value: {
        ...base,
        tracing: { csesTrackId: 'req-review-merged-001' },
        data: {
          id: 'forwarded-merged-001',
          temporaryId: 'relay-req-review-merged-001-0',
          channelId: ids.topic,
          event_seq: 203,
          createAt: 1_700_000_002_000,
          message: '',
          simpleMessage: '[聊天记录]',
          type: 'MULTIPLY',
          userId: ids.actor,
          viewers: ['all'],
          props: {
            forwardTitle: 'Gate Review Group的聊天记录',
            sourceChannelId: ids.channel,
            sourceChannelName: 'Gate Review Group',
            mergeMessage: JSON.stringify(mergedItems),
            channel_event_seq: 203,
          },
        },
        seq: 203,
      },
    },
  ];
}

/** 构造生产 WS parser 当前接受的代表性 wire JSON，并保留 Gate 消息类型差异。 */
function wsSample(action, gateId = undefined) {
  const ids = SAMPLE_IDS;
  const documentMessage =
    gateId === 'G-02'
      ? {
          message: '【文档】 http://doc-pre.cses7.com/doc/review',
          type: 'DOCUMENT',
          props: {
            document: {
              version: 1,
              url: 'http://doc-pre.cses7.com/doc/review',
              title: '打开文档',
              placeholder: true,
            },
            channel_event_seq: 101,
          },
        }
      : {};
  const post = {
    id: ids.message,
    temporaryId: ids.temporary,
    channelId: ids.channel,
    event_seq: 101,
    createAt: 1_700_000_000_000,
    message: 'Gate review message',
    type: 'TEXT',
    userId: ids.actor,
    viewers: ['all'],
    props: { channel_event_seq: 101 },
    ...documentMessage,
  };
  const samples = {
    hello: { action: 'hello', data: { connectionId: 'conn-review-001' } },
    pong: {
      status: 'OK',
      data: {
        text: 'pong',
        gaps: [{ channelId: ids.channel, fromSeq: 100, maxSeq: 101 }],
        hashMismatch: false,
      },
    },
    post: { v: 1, action: 'post', data: post, broadcast: {}, seq: 101 },
    post_update: {
      action: 'post_update',
      data: {
        ...post,
        quickReply: [{ emoji: ':thumbsup:', userIds: [ids.actor] }],
      },
    },
    posts_update: {
      action: 'posts_update',
      data: {
        event_seq: 102,
        event_id: `revoke:${ids.channel}:102`,
        actor_id: ids.actor,
        target_msg_id: ids.message,
        channelId: ids.channel,
        system_event: {
          type: 'system',
          kind: 'system',
          system: true,
          eventId: `revoke:${ids.channel}:102`,
          eventSeq: 102,
          channelId: ids.channel,
          systemEvent: 'message-recalled',
          actorId: ids.actor,
          targetMsgId: ids.message,
          recalledMsgId: ids.message,
          occurredAt: 1_700_000_000_100,
        },
        posts: [
          {
            ...post,
            props: {
              channel_event_seq: 102,
              system_event: {
                type: 'system',
                kind: 'system',
                system: true,
                eventId: `revoke:${ids.channel}:102`,
                eventSeq: 102,
                channelId: ids.channel,
                systemEvent: 'message-recalled',
                actorId: ids.actor,
                targetMsgId: ids.message,
                recalledMsgId: ids.message,
                occurredAt: 1_700_000_000_100,
              },
            },
            revoke: true,
          },
        ],
      },
    },
    revoke_update_channel: {
      action: 'update_channel',
      data: {
        id: ids.channel,
        lastPost: JSON.stringify({
          ...post,
          props: {
            channel_event_seq: 102,
            system_event: {
              type: 'system',
              kind: 'system',
              system: true,
              eventId: `revoke:${ids.channel}:102`,
              eventSeq: 102,
              channelId: ids.channel,
              systemEvent: 'message-recalled',
              actorId: ids.actor,
              targetMsgId: ids.message,
              recalledMsgId: ids.message,
              occurredAt: 1_700_000_000_100,
            },
          },
          revoke: true,
          updateAt: 1_700_000_000_100,
        }),
      },
    },
    post_read: {
      action: 'post_read',
      data: { id: ids.message, channelId: ids.channel, readBits: '11' },
    },
    increment_channel: {
      action: 'increment_channel',
      data: {
        id: ids.channel,
        type: 'P',
        displayName: 'Gate Review Group',
        lastEventSeq: 101,
        members: [{ id: ids.actor }, { id: ids.observer }],
        owner: { id: ids.actor, nickName: 'Actor' },
      },
    },
    increment_channel_end: { action: 'increment_channel_end', data: {} },
    update_channel: {
      action: 'update_channel',
      data: {
        id: ids.channel,
        displayName: 'Gate Review Group',
        unreadCount: 0,
        mentionCount: 0,
        urgentCount: 0,
        lastReadSeq: 101,
        settingVersion: 7,
      },
    },
    update_channel_notice: {
      action: 'update_channel_notice',
      data: { channelId: ids.channel, notice: 'Review notice', settingVersion: 8 },
    },
    channel_created: {
      action: 'channel_created',
      data: {
        id: ids.channel,
        type: 'O',
        displayName: 'Gate Review Group',
        owner: { id: ids.actor },
        members: [{ id: ids.actor }, { id: ids.observer }],
        memberCount: 2,
        createdAt: 1_700_000_000_000,
      },
    },
    channel_close: {
      action: 'channel_close',
      data: { channelId: ids.channel, deleteAt: 1_700_000_000_500 },
    },
    channel_member_update: {
      action: 'channel_member_update',
      data: {
        id: ids.channel,
        type: 'P',
        memberChange: { leave: [{ id: ids.observer }] },
      },
    },
    post_schedule_created: {
      action: 'post_schedule_created',
      eventSeq: 108,
      data: {
        channelId: ids.channel,
        scheduleId: 'schedule-review-001',
        scheduleOwnerUserId: ids.actor,
        message: '下午三点发送',
        messagePreview: '下午三点发送',
        sendAt: 1_700_000_060_000,
        status: 'scheduled',
        revision: 8,
      },
    },
    post_schedule_canceled: {
      action: 'post_schedule_canceled',
      eventSeq: 109,
      data: {
        channelId: ids.channel,
        scheduleId: 'schedule-review-001',
        scheduleOwnerUserId: ids.actor,
        sendAt: 1_700_000_060_000,
        status: 'canceled',
        revision: 9,
      },
    },
  };
  const wire = samples[action];
  if (!wire) {
    throw new Error(`WS sample missing: ${action}`);
  }
  return wire;
}

/** 将 Gate ID 转成测试文件使用的稳定小写前缀。 */
function gatePrefix(gateId) {
  return gateId.toLowerCase().replaceAll('-', '');
}

/** 返回 Gate 所属业务分组，未知 Gate 立即失败而不是静默归入其他。 */
function gateGroup(gateId) {
  const entry = Object.entries(GATE_GROUPS).find(([, ids]) => ids.includes(gateId));
  if (!entry) {
    throw new Error(`Unknown gate group: ${gateId}`);
  }
  return entry[0];
}

/** 递归列出目录下的文件，目录不存在时返回空列表。 */
function walkFiles(directory) {
  if (!existsSync(directory)) {
    return [];
  }
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? walkFiles(path) : [path];
  });
}

/** 判断文件或 Spec 目录名称是否精确属于目标 Gate，避免 G-10 误收 G-10a。 */
function belongsToGate(path, gateId) {
  const normalized = path.toLowerCase().replaceAll('\\', '/');
  const compact = gatePrefix(gateId);
  const dashed = gateId.toLowerCase();
  const compactPattern = new RegExp(`(?:^|[/_-])${compact}(?:[/_.-]|$)`);
  const dashedPattern = new RegExp(`(?:^|[/_-])${dashed}(?:[/_.-]|$)`);
  return compactPattern.test(normalized) || dashedPattern.test(normalized);
}

/** 计算非空文件的 SHA-256；缺失或空文件不构成真实捕获。 */
function captureDigest(path) {
  if (!existsSync(path) || statSync(path).size === 0) {
    return null;
  }
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

/** 以稳定缩进写入 JSON，保证重复生成可比较。 */
function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

/** 读取当前 Git HEAD，作为本地证据合同的源码锁。 */
function currentHead() {
  return execFileSync('git', ['rev-parse', 'HEAD'], {
    cwd: ROOT,
    encoding: 'utf8',
  }).trim();
}

/** 收集 Gate 对应的 Helix producer、fixture 与交互测试源码。 */
function helixSources(gateId) {
  return walkFiles(HELIX_TESTS)
    .filter((path) => belongsToGate(path, gateId))
    .filter((path) => path.endsWith('.rs'))
    .filter((path) => !/(interaction[_-]?frame|surface)/i.test(path))
    .map((path) => relative(ROOT, path))
    .sort();
}

/** 收集 Gate 对应的 WDIO mock、single-real 与 dual-real 源码入口。 */
function wdioSources(gateId) {
  return walkFiles(WDIO_SPECS)
    .filter((path) => belongsToGate(path, gateId))
    .filter((path) => /-(mock|single-real|dual-real)/.test(path))
    .sort();
}

/** 索引历史 Spec evidence 文件但不复制内容或继承其 Green 结论。 */
function historicalEvidence(gateId) {
  return readdirSync(join(ROOT, 'specs'), { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && belongsToGate(entry.name, gateId))
    .flatMap((entry) => walkFiles(join(ROOT, 'specs', entry.name, 'evidence')))
    .filter((path) => !/(interaction[_-]?frame|surface|\.frame-v\d)/i.test(path))
    .map((path) => relative(ROOT, path))
    .sort();
}

/** 将当前源码中的 event 名称转成可审核的 Effect::Emit 合同条目。 */
function emitRecords(gateId) {
  const definition = GATE_EMITS[gateId];
  if (!definition) {
    throw new Error(`Emit contract missing for Gate: ${gateId}`);
  }
  const preexistingGaps = definition.inferred ?? [];
  const messageV3SplitGaps = MESSAGE_V3_SPLIT_GAPS[gateId] ?? [];
  const inferredGaps = [...preexistingGaps, ...messageV3SplitGaps];
  const currentChannels = definition.current.map((event) => ({
    event,
    core_output: 'Effect::Emit',
    envelope: '{event,data}',
    source: EVENT_SOURCES[event] ?? 'MISSING_SOURCE_INDEX',
    source_status: EVENT_SOURCES[event] ? 'CURRENT_SOURCE' : 'MISSING',
  }));
  const targetChannels = (definition.target ?? []).map((target) => ({
    event: target.event,
    core_output: 'Effect::Emit',
    envelope: '{event,data}',
    source: target.source,
    source_status: target.source_status,
  }));
  return {
    channels: [...currentChannels, ...targetChannels],
    required_semantics: definition.required,
    gap_basis: {
      current_runtime_schema: 'crates/helix-im/src/schema.rs',
      message_v3_target_schema:
        'specs/047-im-event-gate-inventory/.v1/本地数据库/完整表结构.sql',
      meaning:
        'inferred_gaps 是当前源码与 MessageV3 目标合同之间的差距；目标表文档完成不等于运行时 Gap 已解决。',
    },
    gap_categories: {
      preexisting: preexistingGaps,
      message_v3_channel_member_split: messageV3SplitGaps,
    },
    inferred_gaps: inferredGaps,
    coverage_status: inferredGaps.length ? 'PARTIAL' : 'CURRENT_SOURCE',
  };
}

/** 读取仓内 F5 canonical Emit，作为消息类样例的真实字段和值基线。 */
function f5EmitSamples() {
  const path = join(HELIX_TESTS, 'fixtures', 'f5', 'f5_golden_events.ndjson');
  const events = readFileSync(path, 'utf8')
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line));
  return Object.fromEntries(events.map((event) => [event.event, event.data]));
}

/** 按生产工厂字段构造一个与本 Gate 入站锚点可关联的 DomainEvent data 样例。 */
function eventSampleData(event, gateId) {
  const ids = SAMPLE_IDS;
  const f5 = f5EmitSamples();
  const sending = {
    ...structuredClone(f5['im:post:sending']),
    channelId: ids.channel,
    channel_id: ids.channel,
    msgId: ids.temporary,
    temporaryId: ids.temporary,
    temporary_id: ids.temporary,
    text: 'Gate review message',
  };
  const received = {
    ...structuredClone(f5['im:post:received']),
    channelId: ids.channel,
    channel_id: ids.channel,
    id: ids.message,
    msgId: ids.message,
    msg_id: ids.message,
    temporaryId: ids.temporary,
    serverId: ids.message,
    userId: ids.actor,
    message: 'Gate review message',
    text: 'Gate review message',
  };
  if (gateId === 'G-02') {
    const document = {
      document: {
        version: 1,
        url: 'http://doc-pre.cses7.com/doc/review',
        title: '打开文档',
        placeholder: true,
      },
    };
    sending.type = 'DOCUMENT';
    sending.text = '【文档】 http://doc-pre.cses7.com/doc/review';
    received.type = 'DOCUMENT';
    received.message = '【文档】 http://doc-pre.cses7.com/doc/review';
    received.text = received.message;
    received.props = document;
  }
  if (gateId === 'G-07') {
    received.urgent = true;
    received.urgentRequesterId = ids.actor;
    received.urgentTargetIds = [ids.observer];
    received.urgentTargetCount = 1;
    received.urgentConfirmedIds = [ids.observer];
    received.urgentConfirmedCount = 1;
    received.urgentRequiredCount = 1;
    received.urgentState = 'completed';
  }
  if (gateId === 'G-14a') {
    received.type = 'TEMPLATE';
    received.props = { template: { userIds: [ids.observer] } };
    received.templateReceived = true;
    received.templateConfirmedCount = 1;
    received.readerIds = [ids.observer];
  }
  const samples = {
    'im:post:sending': sending,
    'im:post:received': received,
    'im:post:updated': {
      ...received,
      quickReply: [{ emoji: ':thumbsup:', userIds: [ids.actor] }],
      updateAt: 1_700_000_000_100,
    },
    'im:post:deleted': {
      ...received,
      type: 'system',
      kind: 'system',
      system: true,
      systemEvent: 'message-recalled',
      eventId: `revoke:${ids.channel}:101`,
      actorId: ids.actor,
      actorAvailable: true,
      targetMsgId: ids.message,
      recalledMsgId: ids.message,
      occurredAt: 1_700_000_000_100,
      displayText: '你撤回了一条消息',
      previewText: '你撤回了一条消息',
      recalledText: 'Gate review message',
      subjectMemberIds: [],
    },
    'im:post:read': { ...received, receiptRevision: 7 },
    'im:channel:read_echo': received,
    'im:post:send-failed': {
      channelId: ids.channel,
      temporaryId: ids.temporary,
      sendStatus: 'failed',
      serverId: null,
      isSelf: true,
      actionCapabilities: {
        canReply: false,
        canForward: false,
        canReact: false,
        canUrgent: false,
        canCreateTopic: false,
        canRevoke: false,
        canRetry: true,
        canUrgentConfirm: false,
        canMarkRead: false,
        canViewReceipt: false,
        canTemplateConfirm: false,
        canLocate: false,
      },
    },
    'im:post:batch-updated': { channel_id: ids.channel, posts: [received] },
    'im:posts:batch-result': {
      reqId: ids.req,
      batchStatus: 'success',
      acceptedCount: 1,
      failedCount: 0,
      deliveryAuthority: 'ws-post',
      targets: [
        {
          channelId: ids.topic,
          acceptanceStatus: 'accepted',
          deliveryStatus: 'pending',
          error: null,
        },
      ],
    },
    'im:sync:state': {
      state: 'committed',
      recovery: { channelId: ids.channel, committedSeq: 101 },
      ...(gateId === 'G-S3' ? { trigger: 'pong_gap' } : {}),
    },
    'im:sync:too_long': {
      channelId: ids.channel,
      resetTo: 101,
      mode: 'drop_and_reload',
      dropLocalMessages: true,
    },
    'im:channels:loaded': { items: [] },
    'im:channel:increment': {
      channel_id: ids.channel,
      increment: wsSample('increment_channel').data,
    },
    'im:channel:update': { channel_id: ids.channel },
    'im:operation:status': {
      reqId: ids.req,
      kind: 'topic-create',
      phase: 'remote-succeeded',
      rootMessageId: ids.message,
      topicChannelId: ids.topic,
      failureCode: null,
      retryable: false,
    },
    'im:messages:query_result': { channel_id: ids.channel, messages: [received] },
    'im:read:result': {
      req_id: ids.req,
      body: { status: 'SUCCESS', data: { posts: [received] } },
    },
    'im:channel:replies': {
      reqId: ids.req,
      channelId: ids.channel,
      rootMessageId: ids.message,
      projectionMode: 'snapshot',
      revision: 7,
      replyIds: ['reply-review-001'],
      nodes: [{ ...received, id: 'reply-review-001', msgId: 'reply-review-001' }],
      replyCount: 1,
      cursor: null,
      hasMore: false,
    },
    'im:channel:members': {
      channelId: ids.channel,
      members: [
        { memberId: ids.actor, nickname: 'Actor', admin: true },
        { memberId: ids.observer, nickname: 'Observer', admin: false },
      ],
      leaves: [],
    },
    'im:post:readers': {
      reqId: ids.req,
      receipts: [
        {
          postId: ids.message,
          state: 'completed',
          snapshotId: 'snapshot-review-001',
          snapshotVersion: 7,
          readerCount: 1,
          unreadCount: 0,
          recipientUserIds: [ids.observer],
          readBits: '1',
          authorExcluded: true,
          contractVersion: 2,
          readerIds: [ids.observer],
          unreadIds: [],
          readers: [{ userId: ids.observer }],
          unreaders: [],
        },
      ],
    },
    'im:channel:created': {
      id: ids.channel,
      channelId: ids.channel,
      channel_id: ids.channel,
      displayName: 'Gate Review Group',
      group: 'group',
      channelType: 'O',
      channelKind: 'group',
      memberCount: 2,
      createdAt: 1_700_000_000_000,
      lastActivityAt: 1_700_000_000_000,
      top: false,
      projectionSource: 'channel_created',
      props: wsSample('channel_created').data,
      channel: wsSample('channel_created').data,
    },
    'im:channel:closed': {
      id: ids.channel,
      channelId: ids.channel,
      closed: true,
      deleteAt: 1_700_000_000_500,
    },
    'im:channel:left': { id: ids.channel, channelId: ids.channel, left: true },
    'im:channel:member-updated': {
      channel_id: ids.channel,
      channel: wsSample('channel_member_update').data,
    },
    'im:channel:settings-updated': {
      channelId: ids.channel,
      settings: { displayName: 'Gate Review Renamed', notice: 'Review notice' },
      settingVersion: 8,
      channelCapabilities: { canEdit: true, canManageMembers: true },
    },
    'im:channel:schedule-created': {
      id: ids.channel,
      channelId: ids.channel,
      hasSchedulePost: true,
      scheduleId: 'schedule-review-001',
      sendAt: 1_700_000_060_000,
      scheduleMessage: '下午三点发送',
      messagePreview: '下午三点发送',
      scheduleMessagePreview: '下午三点发送',
      scheduleSendAt: 1_700_000_060_000,
      status: 'scheduled',
      revision: 8,
      scheduleRevision: 8,
      scheduleStatus: 'scheduled',
    },
    'im:channel:schedule-canceled': {
      id: ids.channel,
      channelId: ids.channel,
      hasSchedulePost: false,
      scheduleId: 'schedule-review-001',
      sendAt: 1_700_000_060_000,
      scheduleMessage: null,
      messagePreview: null,
      scheduleMessagePreview: null,
      scheduleSendAt: 1_700_000_060_000,
      status: 'canceled',
      revision: 9,
      scheduleRevision: 9,
      scheduleStatus: 'canceled',
    },
  };
  const sample = samples[event];
  if (!sample) {
    throw new Error(`Event sample missing: ${event}`);
  }
  return sample;
}

/** 将 G01 收敛为最小领域事件字段，去掉能力、别名和可推导状态。 */
function g01EventSampleData(event) {
  const ids = SAMPLE_IDS;
  const samples = {
    'im:post:sending': {
      channelId: ids.channel,
      temporaryId: ids.temporary,
      sendStatus: 'sending',
      text: 'Gate review message',
      type: 'TEXT',
    },
    'im:post:received': {
      id: ids.message,
      channelId: ids.channel,
      temporaryId: ids.temporary,
      eventSeq: 101,
      userId: ids.actor,
      message: 'Gate review message',
      type: 'TEXT',
      createAt: 1_700_000_000_000,
      sendStatus: 'sent',
    },
    'im:post:send-failed': {
      channelId: ids.channel,
      temporaryId: ids.temporary,
      sendStatus: 'failed',
    },
  };
  const sample = samples[event];
  if (!sample) {
    throw new Error(`G01 event sample missing: ${event}`);
  }
  return sample;
}

/** 生成消息 Gate 写后读回的 viewer-local channel 绝对态，不把 delta 交给壳计算。 */
function postChannelUpdateData(gateId, unreadCount) {
  const post =
    gateId === 'G-01'
      ? g01EventSampleData('im:post:received')
      : eventSampleData('im:post:received', gateId);
  return {
    channelId: SAMPLE_IDS.channel,
    lastPost: {
      id: post.id,
      userId: post.userId,
      message: post.message,
      createAt: post.createAt,
    },
    unreadCount,
  };
}

/** 构造 G-05 两种转发模式在目标频道持久化后的最小消息终态。 */
function g05ForwardedPostData(mode, index = 0) {
  const ids = SAMPLE_IDS;
  if (mode === 'merged') {
    const items = g05MergedItems().map((item) => ({
      id: item.id,
      userId: item.userId,
      author: item.userSnapshot.userName,
      text: item.message,
      type: item.type,
      createAt: item.createAt,
      userSnapshot: item.userSnapshot,
    }));
    return {
      id: 'forwarded-merged-001',
      channelId: ids.topic,
      temporaryId: 'relay-req-review-merged-001-0',
      eventSeq: 203,
      userId: ids.actor,
      message: '',
      type: 'MULTIPLY',
      createAt: 1_700_000_002_000,
      sendStatus: 'sent',
      forwardDetail: {
        mode: 'merged',
        state: 'ready',
        title: 'Gate Review Group的聊天记录',
        summary: '共2条消息',
        itemCount: 2,
        sourceChannelId: ids.channel,
        sourceChannelName: 'Gate Review Group',
        items,
        failureReason: null,
      },
    };
  }
  const source = [
    { id: 'forwarded-individual-001', message: 'Forwarded message A', eventSeq: 201 },
    { id: 'forwarded-individual-002', message: 'Forwarded message B', eventSeq: 202 },
  ][index];
  return {
    id: source.id,
    channelId: ids.topic,
    temporaryId: `relay-req-review-individual-001-${index}`,
    eventSeq: source.eventSeq,
    userId: ids.actor,
    message: source.message,
    type: 'TEXT',
    createAt: 1_700_000_001_000 + index,
    sendStatus: 'sent',
  };
}

/** 构造 G-05 发起者可见的 HTTP 接受结果，明确 accepted 仍等待 WS post。 */
function g05BatchResultData(mode) {
  const reqId =
    mode === 'merged' ? 'req-review-merged-001' : 'req-review-individual-001';
  return {
    reqId,
    batchStatus: 'success',
    acceptedCount: 1,
    failedCount: 0,
    deliveryAuthority: 'ws-post',
    targets: [
      {
        channelId: SAMPLE_IDS.topic,
        acceptanceStatus: 'accepted',
        deliveryStatus: 'pending',
        error: null,
      },
    ],
  };
}

/** 构造 G-05 目标频道写后 lastPost/unreadCount 绝对态。 */
function g05ChannelUpdateData(post, perspective) {
  return {
    channelId: SAMPLE_IDS.topic,
    lastPost: {
      id: post.id,
      userId: post.userId,
      message: post.type === 'MULTIPLY' ? '[聊天记录]' : post.message,
      createAt: post.createAt,
    },
    unreadCount: perspective === 'receiver' ? 1 : 0,
  };
}

/** 按模式构造 G-05 的 sender/receiver 出站序列，避免两种转发混成一条流程。 */
function g05OutboundByMode() {
  const individual = [g05ForwardedPostData('individual', 0), g05ForwardedPostData('individual', 1)];
  const merged = g05ForwardedPostData('merged');
  return {
    individual: {
      sender: [
        { event: 'im:posts:batch-result', data: g05BatchResultData('individual') },
        ...individual.map((post) => ({ event: 'im:post:received', data: post })),
        {
          event: 'im:channel:update',
          data: g05ChannelUpdateData(individual[1], 'sender'),
        },
      ],
      receiver: [
        ...individual.map((post) => ({ event: 'im:post:received', data: post })),
        {
          event: 'im:channel:update',
          data: g05ChannelUpdateData(individual[1], 'receiver'),
        },
      ],
    },
    merged: {
      sender: [
        { event: 'im:posts:batch-result', data: g05BatchResultData('merged') },
        { event: 'im:post:received', data: merged },
        {
          event: 'im:channel:update',
          data: g05ChannelUpdateData(merged, 'sender'),
        },
      ],
      receiver: [
        { event: 'im:post:received', data: merged },
        {
          event: 'im:channel:update',
          data: g05ChannelUpdateData(merged, 'receiver'),
        },
      ],
    },
  };
}

/** 合并 G-05 两种模式，供 Gate 总览样例继续提供完整 event 并集。 */
function g05OutboundPerspectives() {
  const byMode = g05OutboundByMode();
  return {
    sender: [...byMode.individual.sender, ...byMode.merged.sender],
    receiver: [...byMode.individual.receiver, ...byMode.merged.receiver],
  };
}

/** 生成单一 G-05 模式的双客户端入栈/出栈流程，供人工逐段审阅。 */
function g05FlowSample(mode, sourceLock) {
  const command = g05CommandSamples().find((sample) => sample.payload.mode === mode);
  const authorities = g05AuthoritySamples().filter((sample) =>
    sample.sample_id.includes(mode),
  );
  const outbound = g05OutboundByMode()[mode];
  const inboundEntry = (sample) => ({
    tick_variant: 'Tick::Inbound',
    value: structuredClone(sample.value),
  });
  const causalOrder =
    mode === 'individual'
      ? {
          sender: [
            '1. inbound[0]: 壳提交 im_relay_messages(mode=individual, 2 post_ids)',
            '2. outbound[0]: HTTP accepted 后 emit batch-result；deliveryStatus 仍是 pending',
            '3. inbound[1]: 收到目标消息 A 的 WS post echo',
            '4. outbound[1]: 持久提交消息 A 后 emit im:post:received',
            '5. inbound[2]: 收到目标消息 B 的 WS post echo',
            '6. outbound[2]: 持久提交消息 B 后 emit im:post:received',
            '7. outbound[3]: 持久读回目标频道后 emit lastPost=B、unreadCount=0',
          ],
          receiver: [
            '1. inbound[0]: 收到目标消息 A 的 WS post',
            '2. outbound[0]: 持久提交消息 A 后 emit im:post:received',
            '3. inbound[1]: 收到目标消息 B 的 WS post',
            '4. outbound[1]: 持久提交消息 B 后 emit im:post:received',
            '5. outbound[2]: 持久读回目标频道后 emit lastPost=B、unreadCount=1',
          ],
        }
      : {
          sender: [
            '1. inbound[0]: 壳提交 im_relay_messages(mode=merged, 2 post_ids)',
            '2. outbound[0]: HTTP accepted 后 emit batch-result；deliveryStatus 仍是 pending',
            '3. inbound[1]: 收到唯一 MULTIPLY 消息的 WS post echo',
            '4. outbound[1]: 持久提交后 emit 一条 im:post:received，forwardDetail.items=2',
            '5. outbound[2]: 持久读回目标频道后 emit lastPost=[聊天记录]、unreadCount=0',
          ],
          receiver: [
            '1. inbound[0]: 收到唯一 MULTIPLY 消息的 WS post',
            '2. outbound[0]: 持久提交后 emit 一条 im:post:received，forwardDetail.items=2',
            '3. outbound[1]: 持久读回目标频道后 emit lastPost=[聊天记录]、unreadCount=1',
          ],
        };
  return {
    schema_version: 'helix-im-g05-forward-flow-sample/v1',
    gate_id: 'G-05',
    mode,
    runtime_capture: false,
    source_lock: sourceLock,
    mode_difference:
      mode === 'individual'
        ? '2 条来源消息 -> 2 条独立 TEXT 目标消息'
        : '2 条来源消息 -> 1 条 MULTIPLY 目标消息；两条来源内容放在 forwardDetail.items',
    causal_order: causalOrder,
    sender: {
      inbound: [
        {
          tick_variant: 'Tick::Command',
          value: {
            name: 'im_relay_messages',
            payload: structuredClone(command.payload),
          },
        },
        ...authorities.map(inboundEntry),
      ],
      outbound: structuredClone(outbound.sender),
    },
    receiver: {
      inbound: authorities.map(inboundEntry),
      outbound: structuredClone(outbound.receiver),
    },
    completion_rule:
      'im:posts:batch-result 只表示 HTTP accepted；双方收到同一目标 post 的 WS authority、持久提交并 emit im:post:received 后才算消息可见。',
  };
}

/** 只复制白名单字段，避免把工厂样例中的兼容别名和 UI 能力带入审阅合同。 */
function pickDefined(source, keys) {
  return Object.fromEntries(
    keys.filter((key) => source[key] !== undefined).map((key) => [key, structuredClone(source[key])]),
  );
}

/** 将 message-row 样例收敛为身份、内容、顺序和终态字段。 */
function minimalPostData(source, extraKeys = []) {
  return pickDefined(source, [
    'id',
    'channelId',
    'temporaryId',
    'eventSeq',
    'userId',
    'message',
    'type',
    'createAt',
    'sendStatus',
    ...extraKeys,
  ]);
}

/** 给查询类 Gate 补齐其本地窗口动作的审阅目标状态。 */
function queryResultSampleData(gateId, perspective) {
  const post = minimalPostData(eventSampleData('im:post:received', gateId));
  const modeByGate = {
    'G-10': 'latest',
    'G-10a': 'older',
    'G-10b': 'newer',
    'G-10c': 'append',
    'G-10d': perspective === 'receiver' ? 'append-away-bottom' : 'append-latest',
    'G-11': 'locate',
    'G-11a': 'locate',
  };
  const data = {
    channelId: SAMPLE_IDS.channel,
    mode: modeByGate[gateId] ?? 'latest',
    revision: 7,
    messages: [post],
    bounds: { firstEventSeq: 101, lastEventSeq: 101 },
    olderToken: gateId === 'G-10b' ? null : 'older-review-token',
    newerToken: ['G-10', 'G-10a', 'G-10c'].includes(gateId) ? null : 'newer-review-token',
  };
  if (gateId === 'G-10c') {
    data.evictedHeadCount = 1;
  }
  if (gateId === 'G-10d') {
    data.anchorPreserved = perspective === 'receiver';
    data.newMessageCount = perspective === 'receiver' ? 1 : 0;
  }
  if (gateId === 'G-11' || gateId === 'G-11a') {
    data.targetMessageId = SAMPLE_IDS.message;
  }
  return data;
}

/** 按 Gate 语义生成 channel 更新的绝对态；无现成绝对态时只保留路由身份。 */
function channelUpdateSampleData(gateId, perspective) {
  const postDriven = new Set(['G-01', 'G-02', 'G-03a', 'G-05', 'G-10c', 'G-10d']);
  if (postDriven.has(gateId)) {
    return postChannelUpdateData(gateId, perspective === 'receiver' ? 1 : 0);
  }
  if (gateId === 'G-04') {
    return g04ChannelUpdateData(perspective);
  }
  if (gateId === 'G-13') {
    return {
      channelId: SAMPLE_IDS.channel,
      unreadCount: 0,
      mentionCount: 0,
      urgentCount: 0,
    };
  }
  if (gateId === 'G-07') {
    return {
      channelId: SAMPLE_IDS.channel,
      urgentCount: 0,
    };
  }
  if (gateId === 'G-16c') {
    return { channelId: SAMPLE_IDS.channel, settingVersion: 8 };
  }
  return { channelId: SAMPLE_IDS.channel };
}

/** 生成 G04 本地 message 行可直接读回的撤回事实，不混入展示文案。 */
function g04RevokeData(_perspective) {
  return {
    id: SAMPLE_IDS.message,
    temporaryId: SAMPLE_IDS.temporary,
    channelId: SAMPLE_IDS.channel,
    eventSeq: 102,
    userId: SAMPLE_IDS.actor,
    type: 'TEXT',
    message: 'Gate review message',
    createAt: 1_700_000_000_000,
    updateAt: 1_700_000_000_100,
    revoke: true,
  };
}

/** 生成 G04 撤回最后一条后的 channel_member 绝对态；撤回本身不新增未读。 */
function g04ChannelUpdateData(perspective) {
  const revoke = g04RevokeData(perspective);
  return {
    channelId: SAMPLE_IDS.channel,
    lastPost: {
      id: revoke.id,
      temporaryId: revoke.temporaryId,
      channelId: revoke.channelId,
      userId: revoke.userId,
      message: revoke.message,
      type: revoke.type,
      createAt: revoke.createAt,
      updateAt: revoke.updateAt,
      revoke: revoke.revoke,
    },
    unreadCount: perspective === 'receiver' ? 1 : 0,
  };
}

/** 将各类业务 Emit 数据收敛成无能力字段、无重复别名的 Gate 审阅样例。 */
function minimalEventSampleData(event, gateId, perspective) {
  if (event === 'im:post:revoke') {
    return g04RevokeData(perspective);
  }
  const data = eventSampleData(event, gateId);
  switch (event) {
    case 'im:post:sending':
      return pickDefined(data, ['channelId', 'temporaryId', 'sendStatus', 'text', 'type']);
    case 'im:post:received':
      return minimalPostData(data, gateId === 'G-02' ? ['props'] : []);
    case 'im:post:updated':
      const updateIdentity = ['id', 'channelId', 'eventSeq', 'updateAt'];
      if (gateId === 'G-06') {
        return pickDefined(data, [...updateIdentity, 'quickReply']);
      }
      if (gateId === 'G-07') {
        return pickDefined(data, [
          ...updateIdentity,
          'urgent',
          'urgentRequesterId',
          'urgentTargetIds',
          'urgentConfirmedIds',
          'urgentState',
        ]);
      }
      if (gateId === 'G-14a') {
        return pickDefined(data, [
          ...updateIdentity,
          'templateReceived',
          'templateConfirmedCount',
          'readerIds',
          'props',
        ]);
      }
      return pickDefined(data, updateIdentity);
    case 'im:post:deleted':
      return pickDefined(data, [
        'channelId',
        'eventSeq',
        'eventId',
        'actorId',
        'targetMsgId',
        'occurredAt',
        'displayText',
      ]);
    case 'im:post:read':
      return pickDefined(data, [
        'id',
        'channelId',
        'eventSeq',
        'readBits',
        'readerIds',
        'receiptRevision',
      ]);
    case 'im:channel:read_echo':
      return pickDefined(data, ['id', 'channelId', 'eventSeq', 'readBits', 'readerIds']);
    case 'im:post:send-failed':
      return pickDefined(data, ['channelId', 'temporaryId', 'sendStatus']);
    case 'im:post:batch-updated':
      return {
        channelId: SAMPLE_IDS.channel,
        posts: data.posts.map((post) =>
          pickDefined(post, ['id', 'channelId', 'eventSeq', 'updateAt']),
        ),
      };
    case 'im:posts:batch-result':
      return pickDefined(data, [
        'reqId',
        'batchStatus',
        'acceptedCount',
        'failedCount',
        'deliveryAuthority',
        'targets',
      ]);
    case 'im:sync:state':
      return pickDefined(data, ['state', 'recovery', 'trigger']);
    case 'im:sync:too_long':
      return pickDefined(data, ['channelId', 'resetTo', 'mode', 'dropLocalMessages']);
    case 'im:channels:loaded':
      return pickDefined(data, ['items']);
    case 'im:channel:increment':
      return { channelId: SAMPLE_IDS.channel, increment: data.increment };
    case 'im:channel:update':
      return channelUpdateSampleData(gateId, perspective);
    case 'im:operation:status':
      return {
        reqId: SAMPLE_IDS.req,
        kind: gateId === 'G-08' ? 'schedule-create' : 'schedule-cancel',
        phase: 'completed',
        failureCode: null,
        retryable: false,
      };
    case 'im:messages:query_result':
      return queryResultSampleData(gateId, perspective);
    case 'im:read:result':
      return {
        reqId: SAMPLE_IDS.req,
        status: data.body.status,
        posts: data.body.data.posts.map((post) => minimalPostData(post)),
      };
    case 'im:channel:replies':
      return {
        ...pickDefined(data, [
          'reqId',
          'channelId',
          'rootMessageId',
          'projectionMode',
          'revision',
          'replyIds',
          'replyCount',
          'cursor',
          'hasMore',
        ]),
        nodes: data.nodes.map((post) => minimalPostData(post)),
      };
    case 'im:channel:members':
      if (gateId === 'G-16b' && perspective === 'sender') {
        return {
          channelId: data.channelId,
          members: data.members.filter((member) => member.memberId !== SAMPLE_IDS.observer),
          leaves: [SAMPLE_IDS.observer],
        };
      }
      return pickDefined(data, ['channelId', 'members', 'leaves']);
    case 'im:post:readers':
      return {
        reqId: data.reqId,
        receipts: data.receipts.map((receipt) =>
          pickDefined(receipt, ['postId', 'state', 'readBits', 'readerIds', 'unreadIds']),
        ),
      };
    case 'im:channel:created':
      return pickDefined(data, [
        'channelId',
        'displayName',
        'channelType',
        'memberCount',
        'createdAt',
      ]);
    case 'im:channel:closed':
      return pickDefined(data, ['channelId', 'closed', 'deleteAt']);
    case 'im:channel:left':
      return pickDefined(data, ['channelId', 'left']);
    case 'im:channel:member-updated':
      return {
        channelId: SAMPLE_IDS.channel,
        memberChange: data.channel.memberChange,
      };
    case 'im:channel:settings-updated':
      return pickDefined(data, ['channelId', 'settings', 'settingVersion']);
    case 'im:channel:schedule-created':
    case 'im:channel:schedule-canceled':
      return {
        channelId: data.channelId,
        scheduleId: data.scheduleId,
        sendAt: data.sendAt,
        message: data.scheduleMessage,
        status: data.status,
        revision: data.revision,
      };
    default:
      throw new Error(`Minimal outbound sample missing: ${gateId} ${event}`);
  }
}

/** 按 Gate 定义 sender/receiver 各自真正可见的 action event 集合。 */
function gatePerspectiveEvents(gateId, events) {
  const senderOnly = new Set([
    'G-S3',
    'G-08',
    'G-09',
    'G-10',
    'G-10a',
    'G-10b',
    'G-11',
    'G-11a',
    'G-13',
  ]);
  if (senderOnly.has(gateId)) {
    return { sender: events, receiver: [] };
  }
  if (['G-01', 'G-02', 'G-03a'].includes(gateId)) {
    return {
      sender: events,
      receiver: events.filter(
        (event) => event !== 'im:post:sending' && event !== 'im:post:send-failed',
      ),
    };
  }
  if (gateId === 'G-05') {
    return {
      sender: events,
      receiver: events.filter((event) => event !== 'im:posts:batch-result'),
    };
  }
  if (gateId === 'G-16') {
    return {
      sender: events,
      receiver: events.filter((event) => event === 'im:channel:closed'),
    };
  }
  if (gateId === 'G-14') {
    return {
      sender: events.filter((event) => event === 'im:post:read'),
      receiver: events.filter((event) => event === 'im:post:readers'),
    };
  }
  if (gateId === 'G-16b') {
    return {
      sender: events.filter((event) => event !== 'im:channel:left'),
      receiver: events.filter((event) => event === 'im:channel:left'),
    };
  }
  return { sender: events, receiver: events };
}

/** 判断该 Gate 是否声明了 HTTP PortReply 延续输入。 */
function gateHasHttp(uc) {
  return (uc.http?.response ?? 'N/A') !== 'N/A';
}

/** 生成当前 Gate 共用的 HTTP 成功、错误与超时延续输入。 */
function portContinuationInputs(uc) {
  if (!gateHasHttp(uc)) {
    return {};
  }
  const correlation = `<${uc.commands[0]?.name ?? uc.gate_id} correlation>`;
  const sharedSources = ['crates/helix-core/src/tick.rs'];
  return {
    http_accepted: {
      sample_id: 'http-accepted',
      tick_variant: 'Tick::PortReply',
      rust_shape: 'Tick::PortReply { corr, outcome: PortOutcome::Ok(ReplyBytes) }',
      value: {
        corr: correlation,
        outcome: {
          variant: 'Ok',
          reply_bytes_decoded: {
            status: 200,
            headers: {},
            body_base64: 'eyJzdGF0dXMiOiJTVUNDRVNTIiwiZGF0YSI6e319',
          },
        },
      },
      source_refs: [
        ...sharedSources,
        ...(uc.gate_id === 'G-01'
          ? [
              'crates/helix-im/tests/f1_send_http_reconcile_test.rs',
              'crates/helix-im/tests/g01_shared/scenario.rs',
            ]
          : ['crates/helix-im/src/port_reply/dispatch_core.rs']),
      ],
    },
    http_error_or_timeout: [
      {
        sample_id: 'http-error',
        tick_variant: 'Tick::PortReply',
        rust_shape:
          'Tick::PortReply { corr, outcome: PortOutcome::Err(PortError::Http(500)) }',
        value: {
          corr: correlation,
          outcome: { variant: 'Err', error: 'Http(500)' },
        },
        source_refs: [
          ...sharedSources,
          ...(uc.gate_id === 'G-01'
            ? ['crates/helix-im/tests/g01_shared/scenario.rs']
            : ['crates/helix-im/src/port_reply/dispatch_core.rs']),
        ],
      },
      {
        sample_id: 'http-timeout',
        tick_variant: 'Tick::PortReply',
        rust_shape:
          'Tick::PortReply { corr, outcome: PortOutcome::Err(PortError::Timeout) }',
        value: {
          corr: correlation,
          outcome: { variant: 'Err', error: 'Timeout' },
        },
        source_refs: [
          ...sharedSources,
          'crates/helix-im/src/port_reply/dispatch_core.rs',
        ],
      },
    ],
  };
}

/** 用稳定键组织同一 Gate 的一个或多个 WS authority 输入。 */
function websocketContinuationInputs(gateId, authorities) {
  const declaredActions = GATE_WS_ACTIONS[gateId] ?? [];
  return Object.fromEntries(
    authorities.map((authority, index) => {
      const declared = declaredActions[index] ?? authority.sample_id;
      const key = `ws_${declared.replaceAll(/[^a-zA-Z0-9]+/g, '_')}`;
      return [key, authority];
    }),
  );
}

/** 生成与 inbound JSON 中直接 WS 键一致的引用路径。 */
function websocketContinuationRefs(gateId) {
  const authorities =
    gateId === 'G-05'
      ? g05AuthoritySamples()
      : (GATE_WS_ACTIONS[gateId] ?? []).map((action, index) => ({
          sample_id: `ws-authority-${index + 1}`,
          value: { action },
        }));
  return Object.keys(websocketContinuationInputs(gateId, authorities)).map(
    (key) => `continuation_inputs.${key}`,
  );
}

/** 生成 Gate 的唯一业务入口及同一状态机的延续权威输入。 */
function inboundSampleRecord(uc, sourceLock) {
  const commands =
    uc.gate_id === 'G-05'
      ? g05CommandSamples().map((sample) => ({
          sample_id: sample.sample_id,
          tick_variant: 'Tick::Command',
          rust_shape: 'AppCommand { name: Cow<str>, payload: Bytes(JSON) }',
          value: { name: 'im_relay_messages', payload: sample.payload },
          source_refs: [
            'crates/helix-core/src/tick.rs',
            'crates/helix-im/src/forward.rs',
            'crates/helix-im/src/forward_shape.rs',
            'crates/helix-im/tests/one_line_server_contracts_test.rs',
          ],
        }))
      : uc.commands.map((command, index) => ({
          sample_id: `command-${index + 1}`,
          tick_variant: 'Tick::Command',
          rust_shape: 'AppCommand { name: Cow<str>, payload: Bytes(JSON) }',
          value: {
            name: command.name,
            payload: commandSamplePayload(command.name, uc.gate_id),
          },
          source_refs: [
            'crates/helix-core/src/tick.rs',
            COMMAND_SAMPLE_SOURCES[command.name] ??
              'crates/helix-im/contracts/strict-matrix.v1.json',
          ],
        }));
  const authoritySamples =
    uc.gate_id === 'G-05'
      ? g05AuthoritySamples()
      : (GATE_WS_ACTIONS[uc.gate_id] ?? []).map((action, index) => ({
          sample_id: `ws-authority-${index + 1}`,
          value: wsSample(action, uc.gate_id),
        }));
  const authorities = authoritySamples.map((sample) => ({
    sample_id: sample.sample_id,
    tick_variant: 'Tick::Inbound',
    rust_shape: 'InboundBytes(Bytes(JSON))',
    value: sample.value,
    source_refs: [
      'crates/helix-core/src/tick.rs',
      'crates/helix-im/src/ws/frame.rs',
      'crates/helix-im/tests/g01_shared/scenario.rs',
      'crates/helix-im/tests/s1_ws_actions_test.rs',
      ...(uc.gate_id === 'G-04'
        ? [
            '/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_post_revoke.go',
            '/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_post_revoke_integration_test.go',
          ]
        : []),
    ],
  }));
  const websocketAuthorities = websocketContinuationInputs(uc.gate_id, authorities);
  return {
    schema_version: 'helix-im-gate-inbound-sample/v1',
    gate_id: uc.gate_id,
    evidence_level: 'SOURCE_DERIVED_SAMPLE',
    runtime_capture: false,
    warning:
      uc.gate_id === 'G-04'
        ? 'entry_input 是唯一业务入口；continuation_inputs 是同一状态机的后续权威输入，不是新 Gate。G04 的 post.revoke/channel_event 是远端持久事实；system_event 是 Go 在事务提交后补入 WS 的传输派生。'
        : 'entry_input 是唯一业务入口；continuation_inputs 是同一状态机的后续权威输入，不是新的 Gate 入口；字段和值来自当前 parser/生产测试结构，不是本轮真实网络捕获。',
    source_lock: sourceLock,
    ...(uc.gate_id === 'G-05'
      ? {
          flow_samples: [
            'individual-forward-flow-sample.json',
            'merged-forward-flow-sample.json',
          ],
        }
      : {}),
    entry_input: commands[0],
    continuation_inputs: {
      ...(commands.length > 1
        ? {
            additional_commands: {
              meaning:
                '同一 Gate 的替代入口或后续业务动作；不新增第二个顶层 entry_input。',
              samples: commands.slice(1),
            },
          }
        : {}),
      ...portContinuationInputs(uc),
      ...(authorities.length > 0 ? websocketAuthorities : {}),
    },
  };
}

/** 按 G01 模型把 Emit 分为前奏、互斥结果与关联投影。 */
function outboundEventPartitions(gateId, events) {
  const eventSet = new Set(events);
  const prelude = (GATE_PRELUDE_EVENTS[gateId] ?? []).filter((event) =>
    eventSet.has(event),
  );
  const failure = (GATE_FAILURE_EVENTS[gateId] ?? []).filter((event) =>
    eventSet.has(event),
  );
  const associated = (GATE_ASSOCIATED_EVENTS[gateId] ?? []).filter((event) =>
    eventSet.has(event),
  );
  const excluded = new Set([...prelude, ...failure, ...associated]);
  return {
    prelude,
    success: events.filter((event) => !excluded.has(event)),
    failure,
    associated,
  };
}

/** 从某一视角样例中选出指定事件集合并保持声明顺序。 */
function selectPerspectiveSamples(samples, events) {
  return events.flatMap((event) =>
    samples.filter((sample) => sample.event === event),
  );
}

/** 给主链结果挂接对应的 PortReply 或 WS authority 引用。 */
function continuationInputRefs(uc, outcome) {
  if (outcome === 'failure') {
    return gateHasHttp(uc) ? ['continuation_inputs.http_error_or_timeout'] : [];
  }
  const authorities = GATE_WS_ACTIONS[uc.gate_id] ?? [];
  if (authorities.length > 0 || uc.gate_id === 'G-05') {
    return websocketContinuationRefs(uc.gate_id);
  }
  return gateHasHttp(uc) ? ['continuation_inputs.http_accepted'] : [];
}

/** 用单数或复数引用字段表达一个或多个 continuation input。 */
function continuationReferenceFields(refs) {
  if (refs.length === 1) {
    return { continuation_input_ref: refs[0] };
  }
  return { continuation_input_refs: refs };
}

/** 生成 Gate 的主链与关联链 sender/receiver Effect::Emit 审阅样例。 */
function outboundSampleRecord(uc, sourceLock) {
  const gateId = uc.gate_id;
  const emits = emitRecords(gateId);
  const events = emits.channels.map((channel) => channel.event);
  const perspectives = gatePerspectiveEvents(gateId, events);
  const g05Perspectives = gateId === 'G-05' ? g05OutboundPerspectives() : null;
  const hasTarget =
    emits.inferred_gaps.length > 0 ||
    emits.channels.some((channel) => channel.source_status === 'TARGET_REQUIRED');
  const eventEnvelope = (event, perspective) => ({
    event,
    data:
      gateId === 'G-01' && event !== 'im:channel:update'
        ? g01EventSampleData(event)
        : minimalEventSampleData(event, gateId, perspective),
  });
  const senderSamples =
    g05Perspectives?.sender ??
    perspectives.sender.map((event) => eventEnvelope(event, 'sender'));
  const receiverSamples =
    g05Perspectives?.receiver ??
    perspectives.receiver.map((event) => eventEnvelope(event, 'receiver'));
  const partitions = outboundEventPartitions(gateId, events);
  return {
    schema_version: 'helix-im-gate-outbound-sample/v1',
    gate_id: gateId,
    evidence_level: hasTarget
      ? 'SOURCE_DERIVED_TARGET_SAMPLE'
      : 'SOURCE_DERIVED_REVIEW_SAMPLE',
    runtime_capture: false,
    sample_shape: 'DOMAIN_EVENT_ENVELOPE_ONLY',
    source_lock: sourceLock,
    ...(gateId === 'G-05'
      ? {
          flow_samples: [
            'individual-forward-flow-sample.json',
            'merged-forward-flow-sample.json',
          ],
        }
      : {}),
    sample_sources: Object.fromEntries(
      emits.channels.map((channel) => [
        channel.event,
        { source: channel.source, source_status: channel.source_status },
      ]),
    ),
    primary_outbound: {
      prelude: {
        sender_samples: selectPerspectiveSamples(senderSamples, partitions.prelude),
        receiver_samples: selectPerspectiveSamples(receiverSamples, partitions.prelude),
      },
      outcomes: {
        success: {
          declared: partitions.success.length > 0,
          ...continuationReferenceFields(continuationInputRefs(uc, 'success')),
          sender_samples: selectPerspectiveSamples(senderSamples, partitions.success),
          receiver_samples: selectPerspectiveSamples(receiverSamples, partitions.success),
        },
        failure: {
          declared: partitions.failure.length > 0,
          ...continuationReferenceFields(
            partitions.failure.length > 0
              ? continuationInputRefs(uc, 'failure')
              : [],
          ),
          sender_samples: selectPerspectiveSamples(senderSamples, partitions.failure),
          receiver_samples: selectPerspectiveSamples(receiverSamples, partitions.failure),
        },
      },
    },
    associated_outbound:
      partitions.associated.length === 0
        ? []
        : [
            {
              attached_to: 'primary_outbound.outcomes.success',
              sender_samples: selectPerspectiveSamples(
                senderSamples,
                partitions.associated,
              ),
              receiver_samples: selectPerspectiveSamples(
                receiverSamples,
                partitions.associated,
              ),
            },
          ],
    gap_basis: emits.gap_basis,
    gap_categories: emits.gap_categories,
    inferred_gaps: emits.inferred_gaps,
  };
}

/** 将 Matrix UC 条目转换为 Tick、内部 Effect 与业务 Emit 三层合同。 */
function contractRecord(uc, sourceLock) {
  const effects = [...new Set(uc.commands.flatMap((command) => command.effect ?? []))];
  const emits = emitRecords(uc.gate_id);
  const tickKinds = ['Tick::Command'];
  if ((uc.ws?.events ?? 'N/A') !== 'N/A' && !String(uc.ws?.events).startsWith('无')) {
    tickKinds.push('Tick::Inbound');
  }
  if ((uc.http?.response ?? 'N/A') !== 'N/A') {
    tickKinds.push('Tick::PortReply');
  }
  if (effects.includes('timer')) {
    tickKinds.push('Tick::Timer');
  }
  return {
    schema_version: 'helix-im-gate-evidence/v1',
    gate_id: uc.gate_id,
    title: uc.title,
    group: gateGroup(uc.gate_id),
    evidence_level: 'DECLARED_CONTRACT',
    direction_basis: 'Helix module boundary',
    source_lock: sourceLock,
    inbound: {
      tick_kinds: tickKinds,
      ui_commands: uc.commands.map((command) => ({
        name: command.name,
        input: command.input,
      })),
      websocket_authority: {
        status: 'DECLARED',
        events: uc.ws?.events ?? 'N/A',
      },
      port_responses: {
        http:
          (uc.http?.response ?? 'N/A') === 'N/A'
            ? 'N/A'
            : 'Tick::PortReply(HttpOk/HttpErr)；原始 body 写入 inbound capture，本目录不继承 Matrix 中的旧 UI 投影描述',
        persistence: 'PersistOk / PersistErr；具体完成屏障以该 Gate producer 测试为准',
      },
    },
    internal_effects: {
      matrix_effect_tags: effects,
      http_request: uc.http?.request ?? 'N/A',
      persistence: uc.persistence ?? [],
    },
    outbound: {
      contract: 'Effect::Emit { event: DomainEventBytes({event,data}) }',
      ...emits,
      adapter_delivery: {
        native: 'EventSink broadcast；Tauri adapter 令 channel=event',
        ffi: '同一步的 DomainEventBytes 通过 on_domain_events_batch 批量交付',
      },
    },
    negative_assertions: (uc.negative_assertions ?? []).filter(
      (assertion) => !/(interaction[_-]?frame|surface|\bframe\b|\bdom\b)/i.test(assertion),
    ),
    omitted_legacy_ui_assertions: (uc.negative_assertions ?? []).filter((assertion) =>
      /(interaction[_-]?frame|surface|\bframe\b|\bdom\b)/i.test(assertion),
    ).length,
  };
}

/** 阻止生成器退回平铺 samples 或在 Emit envelope 中夹带额外字段。 */
function validateOrganizedSamples(inbound, outbound) {
  const forbiddenInbound = [
    'storage_schema_lock',
    'remote_persistence_basis',
    'boundary_rule',
  ];
  const forbiddenOutbound = [
    'storage_schema_lock',
    'sender_local_persistence_basis',
    'receiver_local_persistence_basis',
    'field_lineage',
    'boundary_rule',
    'warning',
  ];
  if (!inbound.entry_input || 'samples' in inbound) {
    throw new Error(`${inbound.gate_id}: inbound 必须只有一个 entry_input，禁止平铺 samples`);
  }
  if (
    !inbound.continuation_inputs ||
    !outbound.primary_outbound ||
    !Array.isArray(outbound.associated_outbound) ||
    'sender_samples' in outbound ||
    'receiver_samples' in outbound
  ) {
    throw new Error(`${inbound.gate_id}: outbound 未按 G01 主链/关联链结构生成`);
  }
  if (forbiddenInbound.some((field) => field in inbound)) {
    throw new Error(`${inbound.gate_id}: inbound 出现已剔除的持久层说明字段`);
  }
  if (forbiddenOutbound.some((field) => field in outbound)) {
    throw new Error(`${inbound.gate_id}: outbound 出现已剔除的持久层说明字段`);
  }
  const sampleGroups = [
    outbound.primary_outbound.prelude.sender_samples,
    outbound.primary_outbound.prelude.receiver_samples,
    outbound.primary_outbound.outcomes.success.sender_samples,
    outbound.primary_outbound.outcomes.success.receiver_samples,
    outbound.primary_outbound.outcomes.failure.sender_samples,
    outbound.primary_outbound.outcomes.failure.receiver_samples,
    ...outbound.associated_outbound.flatMap((item) => [
      item.sender_samples,
      item.receiver_samples,
    ]),
  ];
  for (const sample of sampleGroups.flat()) {
    const keys = Object.keys(sample).sort().join(',');
    if (keys !== 'data,event') {
      throw new Error(
        `${inbound.gate_id}: Emit 样例只能包含 {event,data}，实际为 ${keys}`,
      );
    }
  }
}

/** 建立 Tick、Effect 与 Emit 三个原始捕获槽位。 */
function captureStatus(gateId, sourceIndexes) {
  const captureDir = join(OUTPUT_DIR, 'gates', gateId, 'captures');
  mkdirSync(captureDir, { recursive: true });
  const slots = Object.fromEntries(
    ['inbound', 'effects', 'emit'].map((name) => {
      const path = join(captureDir, `${name}.ndjson`);
      const sha256 = captureDigest(path);
      return [
        name,
        {
          path: relative(join(OUTPUT_DIR, 'gates', gateId), path),
          status: sha256 ? 'CAPTURE_PRESENT_UNVERIFIED' : 'NOT_CAPTURED',
          sha256,
        },
      ];
    }),
  );
  const captured = Object.values(slots).filter(
    (slot) => slot.status === 'CAPTURE_PRESENT_UNVERIFIED',
  ).length;
  return {
    schema_version: 'helix-im-gate-capture-status/v1',
    gate_id: gateId,
    slots,
    source_indexes: sourceIndexes,
    captured_slots: captured,
    required_slots: 3,
    verdict: captured === 3 ? 'CAPTURE_SET_PRESENT_UNVERIFIED' : 'INCOMPLETE',
    warning:
      '源码、fixture、旧 evidence 或测试 PASS 不能替代同一 run 的 inbound/effects/emit 原始捕获。',
  };
}

/** 渲染本地总览，优先展示捕获缺口而不是旧报告的 Green 标签。 */
function renderReadme(records, manifest) {
  const rows = records
    .map(({ contract, status }) => {
      const commands = contract.inbound.ui_commands.map((item) => item.name).join('<br>') || 'N/A';
      const channels =
        contract.outbound.channels.map((item) => item.event).join('<br>') || 'MISSING';
      return `| ${contract.gate_id} | ${contract.group} | ${commands} | ${channels} | ${contract.outbound.coverage_status} | ${status.captured_slots}/3 | ${status.verdict} |`;
    })
    .join('\n');
  return `# IM Gate 本地证据索引 v1

> 本目录由 \`scripts/generate-v1-evidence.mjs\` 生成并被 Git 忽略。它首先盘点合同与
> 源码入口，不把旧日志或静态测试冒充本轮真实网络/Port 捕获。

## 方向定义

- \`inbound\`：进入模块的 \`Tick::Command/Inbound/PortReply/Timer\`。
- \`effects\`：模块交给 host 执行的完整 \`Effect\` 流，用于核对 HTTP、Persist、Timer。
- \`emit\`：对端消费的业务出站，只认 \`Effect::Emit\` 中的 \`{event,data}\`。

每个 Gate 的 \`contract.json\` 是 Matrix 声明；\`capture-status.json\` 是原始捕获状态。
Native/Tauri 的 \`channel=event\` 是适配器路由；FFI 按 step 批量交付 event bytes。
\`inbound-sample.json\` 与 \`outbound-sample.json\` 是当前源码派生的审阅样例，
固定 \`runtime_capture=false\`，不能冒充真实运行证据。
所有 Gate 的 inbound 统一为唯一 \`entry_input\` + \`continuation_inputs\`；
outbound 统一为 \`primary_outbound.prelude/outcomes\` +
\`associated_outbound\`。每个 sender/receiver 项只能是 \`{event,data}\`。
样例文件不再内嵌 schema lock、远端/本地持久层依据、字段 lineage 或边界说明；
数据库依据保留在独立合同与数据库文档中，不污染 Gate 的出入站样例。
真实采集文件只能写入 \`captures/inbound.ndjson\`、\`effects.ndjson\`、
\`emit.ndjson\`。空文件仍按 \`NOT_CAPTURED\` 处理。
全部 Gate 的既有差距与 MessageV3 分表差距见 [GAPS.md](GAPS.md)。

## 汇总

- Gate：${manifest.summary.gates}
- 源码派生审阅样例：${manifest.summary.source_derived_sample_files}
- 必需捕获槽位：${manifest.summary.required_capture_slots}
- 已存在但未验证槽位：${manifest.summary.present_unverified_capture_slots}
- 缺失槽位：${manifest.summary.missing_capture_slots}
- 结论：${manifest.summary.verdict}

| Gate | 分组 | 入站 command | Emit channel | 合同覆盖 | 捕获 | 状态 |
|---|---|---|---|---|---:|---|
${rows}
`;
}

/** 渲染全部 Gate 的既有 Gap 与 MessageV3 分表新增 Gap，供逐项审阅。 */
function renderGapInventory(records) {
  const partial = records.filter(
    ({ contract }) => contract.outbound.coverage_status === 'PARTIAL',
  ).length;
  const splitAffected = records.filter(
    ({ contract }) =>
      contract.outbound.gap_categories.message_v3_channel_member_split.length > 0,
  ).length;
  const sections = records
    .map(({ contract }) => {
      const categories = contract.outbound.gap_categories;
      const renderList = (items) =>
        items.length > 0 ? items.map((item) => `- ${item}`).join('\n') : '- 无。';
      return `## ${contract.gate_id}

- 覆盖状态：\`${contract.outbound.coverage_status}\`

### 原有 Gap

${renderList(categories.preexisting)}

### MessageV3 分表 Gap

${renderList(categories.message_v3_channel_member_split)}
`;
    })
    .join('\n');
  return `# IM Gate Gap 清单

> 本清单比较当前 Helix 源码与 MessageV3 目标合同。
> \`inferred_gaps\` 表示根据源码能够确认的实现差距，不是运行错误日志；
> 修改目标表结构文档不会自动关闭运行时 Gap。

## 汇总

- Gate 总数：${records.length}
- \`PARTIAL\`：${partial}
- \`CURRENT_SOURCE\`：${records.length - partial}
- 受 \`channel/channel_member\` 分表影响：${splitAffected}
- MessageV3 目标结构：
  [本地数据库/完整表结构.sql](本地数据库/完整表结构.sql)

${sections}`;
}

/** 生成 30 Gate 合同、源码索引、捕获状态和本地总 manifest。 */
function main() {
  validateStorageProfiles();
  const matrix = JSON.parse(readFileSync(MATRIX_PATH, 'utf8'));
  const gateOptionIndex = process.argv.indexOf('--gate');
  const requestedGateId =
    gateOptionIndex >= 0 ? process.argv[gateOptionIndex + 1] : undefined;
  if (requestedGateId && !GATE_IDS.includes(requestedGateId)) {
    throw new Error(`Unknown --gate value: ${requestedGateId}`);
  }
  const selectedGateIds = requestedGateId ? [requestedGateId] : GATE_IDS;
  const sourceLock = {
    helix_head: currentHead(),
    matrix_epoch: matrix.contract_epoch,
    matrix_path: relative(ROOT, MATRIX_PATH),
  };
  const ucByGate = new Map(matrix.uc_cases.map((uc) => [uc.gate_id, uc]));
  const records = selectedGateIds.map((gateId) => {
    const uc = ucByGate.get(gateId);
    if (!uc) {
      throw new Error(`Gate missing from Strict Matrix: ${gateId}`);
    }
    const sourceIndexes = {
      helix_tests: helixSources(gateId),
      wdio_sources: wdioSources(gateId),
      historical_evidence: historicalEvidence(gateId),
    };
    const contract = contractRecord(uc, sourceLock);
    const status = captureStatus(gateId, sourceIndexes);
    const gateDir = join(OUTPUT_DIR, 'gates', gateId);
    writeJson(join(gateDir, 'contract.json'), contract);
    writeJson(join(gateDir, 'inbound-contract.json'), {
      schema_version: 'helix-im-gate-inbound-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      ...contract.inbound,
    });
    writeJson(join(gateDir, 'effects-contract.json'), {
      schema_version: 'helix-im-gate-effects-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      ...contract.internal_effects,
    });
    writeJson(join(gateDir, 'outbound-contract.json'), {
      schema_version: 'helix-im-gate-outbound-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      ...contract.outbound,
    });
    const inboundSample = inboundSampleRecord(uc, sourceLock);
    const outboundSample = outboundSampleRecord(uc, sourceLock);
    validateOrganizedSamples(inboundSample, outboundSample);
    writeJson(join(gateDir, 'inbound-sample.json'), inboundSample);
    writeJson(join(gateDir, 'outbound-sample.json'), outboundSample);
    if (gateId === 'G-05') {
      writeJson(
        join(gateDir, 'individual-forward-flow-sample.json'),
        g05FlowSample('individual', sourceLock),
      );
      writeJson(
        join(gateDir, 'merged-forward-flow-sample.json'),
        g05FlowSample('merged', sourceLock),
      );
    }
    rmSync(join(gateDir, 'response-contract.json'), { force: true });
    writeJson(join(gateDir, 'capture-status.json'), status);
    return { contract, status };
  });
  if (requestedGateId) {
    process.stdout.write(
      `${JSON.stringify({ gate_id: requestedGateId, generated: true })}\n`,
    );
    return;
  }
  const present = records.reduce((sum, record) => sum + record.status.captured_slots, 0);
  const manifest = {
    schema_version: 'helix-im-gate-evidence-manifest/v1',
    generated_from: sourceLock,
    direction_basis: 'Helix boundary',
    gate_ids: GATE_IDS,
    summary: {
      gates: records.length,
      source_derived_sample_files: records.length * 2,
      required_capture_slots: records.length * 3,
      present_unverified_capture_slots: present,
      missing_capture_slots: records.length * 3 - present,
      verdict: present === records.length * 3 ? 'CAPTURE_SET_PRESENT_UNVERIFIED' : 'INCOMPLETE',
    },
  };
  writeJson(join(OUTPUT_DIR, 'manifest.json'), manifest);
  writeFileSync(join(OUTPUT_DIR, 'README.md'), renderReadme(records, manifest));
  writeFileSync(join(OUTPUT_DIR, 'GAPS.md'), renderGapInventory(records));
  process.stdout.write(`${JSON.stringify(manifest.summary)}\n`);
}

main();
