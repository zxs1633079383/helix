#!/usr/bin/env python3
"""只读验证 Prometheus metadata 与近期指标样本。"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urljoin
from urllib.request import Request, urlopen


METRIC_NAME = re.compile(r"^[a-zA-Z_:][a-zA-Z0-9_:]*$")
EXIT_PASS = 0
EXIT_INPUT = 2
EXIT_API = 3
EXIT_METADATA = 4
EXIT_EMPTY = 5
EXIT_STALE = 6
EXIT_THRESHOLD = 7


def parse_args() -> argparse.Namespace:
    """解析验证器参数，并拒绝无法形成 Prometheus 查询的输入。"""
    parser = argparse.ArgumentParser(
        description="只读检查 Prometheus metric metadata 与近期非空样本。"
    )
    parser.add_argument("--prom-url", default="http://127.0.0.1:9090")
    parser.add_argument("--metric", required=True, help="Prometheus 最终指标名")
    parser.add_argument(
        "--selector",
        default="",
        help='可选 label matcher，例如 service_name="helix-host-cli"',
    )
    parser.add_argument("--max-age-seconds", type=float, default=120.0)
    parser.add_argument("--clock-skew-seconds", type=float, default=30.0)
    parser.add_argument("--min-value", type=float)
    parser.add_argument("--timeout-seconds", type=float, default=5.0)
    return parser.parse_args()


def emit(payload: dict[str, Any]) -> None:
    """以单个 JSON 对象输出结果，避免把业务 payload 写入终端。"""
    print(json.dumps(payload, ensure_ascii=False, sort_keys=True))


def fail_result(
    metric: str,
    reason: str,
    message: str,
    exit_code: int,
) -> int:
    """输出可机器判断的失败原因并返回稳定退出码。"""
    emit(
        {
            "status": "BLOCKED",
            "reason": reason,
            "metric": metric,
            "message": message,
        }
    )
    return exit_code


def endpoint(base_url: str, path: str, params: dict[str, str]) -> str:
    """构造 Prometheus API URL，并统一处理末尾斜杠。"""
    base = base_url.rstrip("/") + "/"
    return urljoin(base, path.lstrip("/")) + "?" + urlencode(params)


def fetch_json(url: str, timeout_seconds: float) -> tuple[dict[str, Any] | None, str | None]:
    """只读请求 Prometheus JSON API，并隐藏响应正文中的潜在敏感信息。"""
    request = Request(url, headers={"Accept": "application/json"}, method="GET")
    try:
        with urlopen(request, timeout=timeout_seconds) as response:
            body = response.read()
    except HTTPError as error:
        return None, f"HTTP {error.code} from Prometheus API"
    except URLError as error:
        return None, f"Prometheus API unavailable: {error.reason}"
    except TimeoutError:
        return None, "Prometheus API request timed out"

    try:
        payload = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None, "Prometheus API returned non-JSON data"
    if not isinstance(payload, dict):
        return None, "Prometheus API returned a non-object JSON value"
    return payload, None


def api_status(payload: dict[str, Any]) -> tuple[bool, str | None]:
    """检查 Prometheus API 的顶层 status 和可选 error 字段。"""
    if payload.get("status") != "success":
        error = payload.get("error")
        error_type = payload.get("errorType")
        detail = str(error or "unknown Prometheus API error")
        if error_type:
            detail = f"{error_type}: {detail}"
        return False, detail
    return True, None


def metric_query(metric: str, selector: str) -> str:
    """拼接 instant query；selector 只作为 PromQL matcher 传给服务端。"""
    clean_selector = selector.strip()
    if not clean_selector:
        return metric
    if clean_selector.startswith("{") and clean_selector.endswith("}"):
        return metric + clean_selector
    return f"{metric}{{{clean_selector}}}"


def validate_samples(
    result: list[Any],
    now: float,
    max_age_seconds: float,
    clock_skew_seconds: float,
) -> tuple[list[dict[str, float]], str | None]:
    """提取 vector 样本并计算新鲜度，不保留完整 label 或业务字段。"""
    samples: list[dict[str, float]] = []
    for item in result:
        if not isinstance(item, dict):
            continue
        value = item.get("value")
        if not isinstance(value, list) or len(value) < 2:
            continue
        try:
            timestamp = float(value[0])
            numeric_value = float(value[1])
        except (TypeError, ValueError):
            continue
        age = now - timestamp
        if age < -clock_skew_seconds:
            continue
        samples.append({"value": numeric_value, "age_seconds": max(0.0, age)})

    if not samples:
        return [], "Prometheus returned no parseable vector samples"
    if not any(sample["age_seconds"] <= max_age_seconds for sample in samples):
        return samples, "all matching samples are older than max-age-seconds"
    return samples, None


def main() -> int:
    """串联 metadata、instant query 和新鲜度检查，整个流程不写外部状态。"""
    args = parse_args()
    metric = args.metric.strip()
    if not METRIC_NAME.fullmatch(metric):
        return fail_result(metric, "INVALID_METRIC_NAME", "metric name is not valid", EXIT_INPUT)
    if args.max_age_seconds < 0 or args.timeout_seconds <= 0 or args.clock_skew_seconds < 0:
        return fail_result(metric, "INVALID_ARGUMENT", "age, timeout, and clock skew must be non-negative; timeout must be positive", EXIT_INPUT)

    metadata_url = endpoint(
        args.prom_url,
        "/api/v1/metadata",
        {"metric": metric},
    )
    metadata_payload, metadata_error = fetch_json(metadata_url, args.timeout_seconds)
    if metadata_error:
        return fail_result(metric, "COLLECTOR_UNAVAILABLE", metadata_error, EXIT_API)
    assert metadata_payload is not None
    metadata_ok, metadata_status_error = api_status(metadata_payload)
    if not metadata_ok:
        return fail_result(metric, "PROM_METADATA_ERROR", metadata_status_error or "metadata query failed", EXIT_API)
    metadata_data = metadata_payload.get("data")
    metadata_items = metadata_data.get(metric) if isinstance(metadata_data, dict) else None
    if not isinstance(metadata_items, list) or not metadata_items:
        return fail_result(metric, "PROM_METADATA_MISSING", "metadata has no entry for metric", EXIT_METADATA)

    query = metric_query(metric, args.selector)
    query_url = endpoint(args.prom_url, "/api/v1/query", {"query": query})
    query_payload, query_error = fetch_json(query_url, args.timeout_seconds)
    if query_error:
        return fail_result(metric, "COLLECTOR_UNAVAILABLE", query_error, EXIT_API)
    assert query_payload is not None
    query_ok, query_status_error = api_status(query_payload)
    if not query_ok:
        return fail_result(metric, "PROM_QUERY_ERROR", query_status_error or "instant query failed", EXIT_API)

    query_data = query_payload.get("data")
    if not isinstance(query_data, dict) or query_data.get("resultType") != "vector":
        return fail_result(metric, "PROM_QUERY_ERROR", "instant query did not return a vector", EXIT_API)
    result = query_data.get("result")
    if not isinstance(result, list) or not result:
        return fail_result(metric, "PROM_QUERY_EMPTY", "query returned no matching series", EXIT_EMPTY)

    samples, sample_error = validate_samples(
        result,
        time.time(),
        args.max_age_seconds,
        args.clock_skew_seconds,
    )
    if not samples:
        return fail_result(metric, "PROM_QUERY_EMPTY", sample_error or "no parseable samples", EXIT_EMPTY)
    freshest_age = min(sample["age_seconds"] for sample in samples)
    if sample_error:
        return fail_result(metric, "STALE_SAMPLE", sample_error, EXIT_STALE)

    if args.min_value is not None and not any(
        sample["value"] >= args.min_value for sample in samples
    ):
        return fail_result(
            metric,
            "VALUE_THRESHOLD_NOT_MET",
            "no recent sample reached min-value",
            EXIT_THRESHOLD,
        )

    emit(
        {
            "status": "PASS",
            "metric": metric,
            "selector_supplied": bool(args.selector.strip()),
            "metadata_count": len(metadata_items),
            "sample_count": len(samples),
            "freshest_age_seconds": round(freshest_age, 3),
            "evidence": [
                "PROM_METADATA",
                "PROM_QUERY_STATUS_SUCCESS",
                "PROM_QUERY_NON_EMPTY_RECENT_SAMPLE",
            ],
        }
    )
    return EXIT_PASS


if __name__ == "__main__":
    sys.exit(main())
