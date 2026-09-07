#!/usr/bin/env python3
"""将已产生的诊断根 JSONL 有界批量送入 Loki；不生成业务记录。"""
import argparse
import datetime
import json
import sys
import time
import urllib.request

FIELDS = frozenset('schema_version event event_id observed_at producer_service source_revision service_name deployment_environment platform stage result reason tenant_id user_id channel_id login_attempt_id device_session_id connection_id track_id event_key business_event_id attempt_id seq_domain event_seq stream_seq stream_epoch requested_from_seq response_next_seq reset_to path recipient_scope returned_channels sync_kind membership_state epoch_start_seq epoch_end_seq projection_revision membership_rows membership_rows_known redacted events_returned recipients omitted_records expected_known evidence_complete dropped_records'.split())

# 按白名单拒绝扩张，避免普通应用日志或消息正文混入业务诊断。
def validate(line):
    row = json.loads(line)
    if not isinstance(row, dict) or row.get('service_name') != 'helix-im-diagnostics':
        return None
    if set(row) - FIELDS or row.get('schema_version') != 1:
        raise ValueError('diagnostic schema contains unsupported fields')
    if not row.get('event_id') or not row.get('observed_at') or not row.get('event'):
        raise ValueError('diagnostic identity or timestamp missing')
    for key, value in row.items():
        if isinstance(value, (list, dict)):
            raise ValueError('diagnostic value must be scalar')
        if ('seq' in key or key.endswith('_id')) and not isinstance(value, str):
            raise ValueError('diagnostic sequence and identity must remain strings')
    return row

# 精确保留原始纳秒时间，不以采集时刻重写业务发生时间。
def timestamp_ns(value):
    text = value.replace('Z', '+00:00')
    parsed = datetime.datetime.fromisoformat(text)
    if parsed.tzinfo is None:
        raise ValueError('timestamp timezone missing')
    seconds = int(parsed.replace(microsecond=0).timestamp())
    fraction = text.split('.', 1)[1] if '.' in text else ''
    fraction = ''.join(c for c in fraction.split('+')[0].split('-')[0] if c.isdigit())
    return str(seconds * 1_000_000_000 + int((fraction + '000000000')[:9]))

# 重试复用同一正文与时间，返回成功仅证明 Loki 接受，不证明业务送达。
def push(rows, endpoint):
    streams = {}
    for row in rows:
        key = (row.get('deployment_environment', ''), row.get('platform', 'server'))
        streams.setdefault(key, []).append([timestamp_ns(row['observed_at']), json.dumps(row, ensure_ascii=False, separators=(',', ':'))])
    body = json.dumps({'streams': [{'stream': {'service_name': 'helix-im-diagnostics', 'deployment_environment': key[0], 'platform': key[1]}, 'values': sorted(values, key=lambda value: int(value[0]))} for key, values in streams.items()]}).encode()
    for attempt in range(3):
        try:
            request = urllib.request.Request(endpoint, data=body, headers={'Content-Type': 'application/json'})
            with urllib.request.urlopen(request, timeout=5) as response:
                if 200 <= response.status < 300:
                    return
        except Exception:
            if attempt == 2:
                raise
            time.sleep(0.1 * 2 ** attempt)

# 有界流式读取已有文件；过长行直接失败，避免截断形成错误身份。
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('file')
    parser.add_argument('--loki-url', required=True)
    args = parser.parse_args()
    rows, total, ignored = [], 0, 0
    with open(args.file, encoding='utf-8') as source:
        while True:
            line = source.readline(65537)
            if not line:
                break
            if len(line) > 65536:
                raise ValueError('line exceeds 64 KiB')
            try:
                row = validate(line)
            except json.JSONDecodeError:
                ignored += 1
                continue
            if row is None:
                ignored += 1
                continue
            rows.append(row)
            if len(rows) == 64:
                push(rows, args.loki_url)
                total += len(rows)
                rows.clear()
    if rows:
        push(rows, args.loki_url)
        total += len(rows)
    print(json.dumps({'accepted_by_loki': total, 'ignored_non_diagnostic_lines': ignored, 'delivery': 'at_least_once', 'business_verified': False}))

if __name__ == '__main__':
    main()
