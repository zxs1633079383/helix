#!/usr/bin/env python3
"""Fail closed when the InteractionFrame V2 per-Gate index drifts."""

from __future__ import annotations

import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DOC_ROOT = REPO_ROOT / "docs" / "interaction-frame-v2-全量替换-按UC"
FEATURE_ROOT = REPO_ROOT / "specs" / "018-interaction-frame-v2-cutover"
INDEX_PATH = FEATURE_ROOT / "per-gate-spec-index.md"
GLOBAL_TASKS_PATH = FEATURE_ROOT / "tasks.md"

EXPECTED_ROWS = (
    ("GS01", ("GS01",), 21),
    ("GS02", ("GS02",), 24),
    ("GS03", ("GS03",), 27),
    ("GS04", ("GS04",), 30),
    ("G01", ("G01",), 33),
    ("G02", ("G02",), 36),
    ("G04", ("G04", "G04a"), 39),
    ("G05", ("G05",), 42),
    ("G06", ("G06",), 45),
    ("G07", ("G07",), 48),
    ("G08", ("G08",), 51),
    ("G09", ("G09",), 54),
    ("G10", ("G10", "G10a"), 57),
    ("G11", ("G11",), 60),
    ("G12", ("G12",), 63),
    ("G13", ("G13",), 66),
    ("G14", ("G14", "G14a"), 69),
    ("G15", ("G15", "G15a"), 72),
    ("G16", ("G16", "G16b", "G16c"), 75),
)

EXPECTED_FILES = (
    "01-V1当前类图.mmd",
    "02-V2目标类图.mmd",
    "03-V1当前执行流程.mmd",
    "04-V2目标执行流程.mmd",
    "05-用户交互流程.mmd",
    "06-Angular组件与交互合同.md",
    "07-Helix命令与Frame合同.md",
    "08-Go接口与WS推送.md",
    "09-远端PG与本地SQLite.md",
    "10-V1下线与V2验收.md",
    "11-V2切换Spec.md",
    "12-V2切换Plan.md",
    "13-V2切换Tasks.md",
)

MATRIX_RE = re.compile(
    r"<!-- GATE_MATRIX_START -->\n(?P<body>.*?)\n<!-- GATE_MATRIX_END -->",
    re.DOTALL,
)
LINK_RE = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
TASK_RE = re.compile(r"^- \[[ xX]\] (T\d{3})\b", re.MULTILINE)
TASK_RANGE_RE = re.compile(r"`T(\d{3})-T(\d{3})`")


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def parse_matrix(index_text: str, errors: list[str]) -> list[list[str]]:
    match = MATRIX_RE.search(index_text)
    if not match:
        fail(errors, "索引缺少 GATE_MATRIX_START/GATE_MATRIX_END 机器边界")
        return []

    rows: list[list[str]] = []
    for line in match.group("body").splitlines():
        if not line.startswith("| "):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if cells[0] in {"业务 Spec", "---"}:
            continue
        rows.append(cells)
    return rows


def verify_file_counts(errors: list[str]) -> None:
    for filename in EXPECTED_FILES[10:]:
        found = sorted(DOC_ROOT.glob(f"*/{filename}"))
        if len(found) != 19:
            fail(errors, f"{filename} 数量应为 19，实际为 {len(found)}")


def verify_global_tasks(errors: list[str]) -> None:
    task_ids = TASK_RE.findall(GLOBAL_TASKS_PATH.read_text(encoding="utf-8"))
    gate_task_ids = [
        task_id for task_id in task_ids if 21 <= int(task_id[1:]) <= 77
    ]
    expected = [f"T{number:03d}" for number in range(21, 78)]
    if gate_task_ids != expected:
        fail(
            errors,
            "全局 tasks.md 的 Gate checklist 必须连续且唯一覆盖 T021-T077",
        )


def verify_matrix(errors: list[str]) -> None:
    index_text = INDEX_PATH.read_text(encoding="utf-8")
    rows = parse_matrix(index_text, errors)
    if len(rows) != 19:
        fail(errors, f"全维度矩阵行数应为 19，实际为 {len(rows)}")
        return

    actual_gate_ids: list[str] = []
    for row, (uc, gate_ids, first_task) in zip(rows, EXPECTED_ROWS, strict=True):
        if len(row) != 12:
            fail(errors, f"{uc} 矩阵列数应为 12，实际为 {len(row)}")
            continue
        if row[0] != uc:
            fail(errors, f"矩阵顺序漂移：期望 {uc}，实际 {row[0]}")

        actual_gates = tuple(part.strip() for part in row[1].split("/"))
        actual_gate_ids.extend(actual_gates)
        if actual_gates != gate_ids:
            fail(
                errors,
                f"{uc} 机器 Gate 应为 {'/'.join(gate_ids)}，实际为 {row[1]}",
            )

        links = LINK_RE.findall("|".join(row[2:11]))
        if len(links) != 13:
            fail(errors, f"{uc} 应直接索引 01-13 共 13 个文件，实际为 {len(links)}")
        expected_links = [
            f"../../docs/interaction-frame-v2-全量替换-按UC/{uc}/{name}"
            for name in EXPECTED_FILES
        ]
        if links != expected_links:
            fail(errors, f"{uc} 的 01-13 链接集合或顺序漂移")
        for relative_link in links:
            target = (FEATURE_ROOT / relative_link).resolve()
            if not target.is_file():
                fail(errors, f"{uc} 索引目标不存在：{relative_link}")

        range_match = TASK_RANGE_RE.fullmatch(row[11])
        expected_last = first_task + 2
        if not range_match or tuple(map(int, range_match.groups())) != (
            first_task,
            expected_last,
        ):
            fail(
                errors,
                f"{uc} 全局 Task 应为 T{first_task:03d}-T{expected_last:03d}",
            )

        gate_tasks_path = DOC_ROOT / uc / "13-V2切换Tasks.md"
        gate_task_ids = TASK_RE.findall(gate_tasks_path.read_text(encoding="utf-8"))
        expected_task_ids = [
            f"T{number:03d}" for number in range(first_task, expected_last + 1)
        ]
        if gate_task_ids != expected_task_ids:
            fail(
                errors,
                f"{uc}/13-V2切换Tasks.md 应精确映射 {'/'.join(expected_task_ids)}",
            )

    expected_gate_ids = [
        gate_id for _, gate_ids, _ in EXPECTED_ROWS for gate_id in gate_ids
    ]
    if actual_gate_ids != expected_gate_ids:
        fail(errors, "矩阵必须按固定顺序精确覆盖 25 个机器 Gate")
    if len(actual_gate_ids) != 25:
        fail(errors, f"机器 Gate 数量应为 25，实际为 {len(actual_gate_ids)}")
    if {"G03", "G03a"} & set(actual_gate_ids):
        fail(errors, "G03/G03a 不得进入 018 验收矩阵")
    if "G17" in actual_gate_ids:
        fail(errors, "未绑定机器 Gate 的 G17 不得进入 018 验收矩阵")


def main() -> int:
    errors: list[str] = []
    verify_file_counts(errors)
    verify_global_tasks(errors)
    verify_matrix(errors)

    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1

    print(
        "PASS: 19 Spec + 19 Plan + 19 Tasks；"
        "25 Gate；T021-T077 连续唯一且逐 Gate 映射完整"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
