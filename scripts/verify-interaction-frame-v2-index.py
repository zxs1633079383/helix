#!/usr/bin/env python3
"""Fail closed when the V2-only UC documentation index drifts."""

from __future__ import annotations

import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DOC_ROOT = REPO_ROOT / "docs" / "interaction-frame-v2-全量替换-按UC"
FEATURE_ROOT = REPO_ROOT / "specs" / "018-interaction-frame-v2-cutover"
INDEX_PATH = FEATURE_ROOT / "per-gate-spec-index.md"
GLOBAL_TASKS_PATH = FEATURE_ROOT / "tasks.md"
CURRENT_MATRIX_PATH = DOC_ROOT / "01-按UC当前代码矩阵.md"

EXPECTED_ROWS = (
    ("GS01", ("G-S1",), 21),
    ("GS02", ("G-S2",), 24),
    ("GS03", ("G-S3",), 27),
    ("GS04", ("G-S4",), 30),
    ("G01", ("G-01",), 33),
    ("G02", ("G-02",), 36),
    ("G04", ("G-04", "G-04a"), 39),
    ("G05", ("G-05",), 42),
    ("G06", ("G-06",), 45),
    ("G07", ("G-07",), 48),
    ("G08", ("G-08",), 51),
    ("G09", ("G-09",), 54),
    ("G10", ("G-10", "G-10a"), 57),
    ("G11", ("G-11",), 60),
    ("G12", ("G-12",), 63),
    ("G13", ("G-13",), 66),
    ("G14", ("G-14", "G-14a"), 69),
    ("G15", ("G-15", "G-15a"), 72),
    ("G16", ("G-16", "G-16b", "G-16c"), 75),
)

ROOT_DOCS = {
    "AGENTS.md",
    "README.md",
    "00-源码核验与版本.md",
    "01-按UC当前代码矩阵.md",
    "02-V2当前公共架构.md",
    "03-协议版本边界.md",
    "04-验收证据边界.md",
    "05-文档完整性检查.md",
    "06-G02-G15-FrameV2字段与验收总索引.md",
    "07-26Gate三仓纵向闭环总Spec索引.md",
}

UC_DOCS = [
    "01-当前类图.mmd",
    "02-当前执行流程.mmd",
    "03-用户交互流程.mmd",
    "04-Angular组件与交互合同.md",
    "05-Helix命令与Frame合同.md",
    "06-Go接口与WS推送.md",
    "07-远端PG与本地SQLite.md",
    "README.md",
]

G01_DEEP_DOCS = [
    "08-G01全领域字段级类图.mmd",
    "09-G01发送全链路时序图.mmd",
    "10-G01三个Frame与状态纠错.mmd",
    "11-G01字段级数据合同与测试证据.md",
]

G01_OPTIONAL_DOCS = [
    "12-G01-完整主链路.mmd",
    "13-G01-Helix-PC-Mobile-V2验收合同.md",
    "AGENTS.md",
    "handoff.md",
]

DEEP_UCS = {"G02", "G04", "G05", "G06", "G07", "G08", "G10", "G13", "G14", "G15"}

MATRIX_RE = re.compile(
    r"<!-- GATE_MATRIX_START -->\n(?P<body>.*?)\n<!-- GATE_MATRIX_END -->",
    re.DOTALL,
)
LINK_RE = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
TASK_RE = re.compile(r"^- \[[ xX]\] (T\d{3})\b", re.MULTILINE)
TASK_RANGE_RE = re.compile(r"`T(\d{3})-T(\d{3})`")


def parse_matrix(index_text: str, errors: list[str]) -> list[list[str]]:
    match = MATRIX_RE.search(index_text)
    if not match:
        errors.append("索引缺少 GATE_MATRIX_START/GATE_MATRIX_END 机器边界")
        return []

    rows: list[list[str]] = []
    for line in match.group("body").splitlines():
        if not line.startswith("| "):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if cells[0] in {"UC", "---"}:
            continue
        rows.append(cells)
    return rows


def verify_doc_shape(errors: list[str]) -> None:
    """Verify the root, base UC, G01 curated, and selected deep-GN inventories."""
    expected_ucs = {uc for uc, _, _ in EXPECTED_ROWS}
    actual_ucs = {path.name for path in DOC_ROOT.iterdir() if path.is_dir()}
    if actual_ucs != expected_ucs:
        errors.append(
            f"UC 目录集合漂移：期望 {sorted(expected_ucs)}，实际 {sorted(actual_ucs)}"
        )

    actual_root_docs = {path.name for path in DOC_ROOT.iterdir() if path.is_file()}
    if actual_root_docs != ROOT_DOCS:
        errors.append(
            f"根文档集合漂移：期望 {sorted(ROOT_DOCS)}，实际 {sorted(actual_root_docs)}"
        )

    for uc in sorted(expected_ucs):
        files = sorted(path.name for path in (DOC_ROOT / uc).iterdir() if path.is_file())
        allowed_optional_docs = G01_OPTIONAL_DOCS if uc == "G01" else []
        deep_docs = (
            [
                f"08-{uc}全领域字段级类图.mmd",
                f"09-{uc}完整主链路时序图.mmd",
                f"10-{uc}-Frame阶段与接受值.mmd",
                f"11-{uc}字段级数据合同与测试证据.md",
                f"12-{uc}-完整主链路.mmd",
                f"13-{uc}-Helix-PC-Mobile-V2验收合同.md",
                "handoff.md",
            ]
            if uc in DEEP_UCS
            else []
        )
        expected_files = sorted(
            UC_DOCS + (G01_DEEP_DOCS if uc == "G01" else deep_docs)
        )
        missing_files = sorted(set(expected_files) - set(files))
        extra_files = sorted(
            set(files) - set(expected_files) - set(allowed_optional_docs)
        )
        if missing_files or extra_files:
            expected_count = len(expected_files)
            errors.append(
                f"{uc} 应保留 {expected_count} 份当前代码文档："
                f"missing={missing_files}, extra={extra_files}"
            )


def verify_document_links(errors: list[str]) -> None:
    """Require every local Markdown link in the generated tree to resolve."""
    for doc_path in DOC_ROOT.rglob("*.md"):
        doc_text = doc_path.read_text(encoding="utf-8")
        for raw_target in LINK_RE.findall(doc_text):
            target = raw_target.split("#", 1)[0]
            if not target or target.startswith(("http://", "https://", "mailto:")):
                continue
            resolved = (doc_path.parent / target).resolve()
            if not resolved.exists():
                errors.append(
                    f"{doc_path.relative_to(DOC_ROOT)} 本地链接不存在：{raw_target}"
                )


def verify_global_tasks(errors: list[str]) -> None:
    task_ids = TASK_RE.findall(GLOBAL_TASKS_PATH.read_text(encoding="utf-8"))
    gate_task_ids = [
        task_id for task_id in task_ids if 21 <= int(task_id[1:]) <= 77
    ]
    expected = [f"T{number:03d}" for number in range(21, 78)]
    if gate_task_ids != expected:
        errors.append("全局 tasks.md 的历史 Gate checklist 必须连续且唯一覆盖 T021-T077")


def verify_matrix(errors: list[str]) -> None:
    rows = parse_matrix(INDEX_PATH.read_text(encoding="utf-8"), errors)
    if len(rows) != 19:
        errors.append(f"当前源码矩阵行数应为 19，实际为 {len(rows)}")
        return

    actual_gate_ids: list[str] = []
    current_matrix = CURRENT_MATRIX_PATH.read_text(encoding="utf-8")
    for row, (uc, gate_ids, first_task) in zip(rows, EXPECTED_ROWS, strict=True):
        if len(row) != 4:
            errors.append(f"{uc} 矩阵列数应为 4，实际为 {len(row)}")
            continue
        if row[0] != uc:
            errors.append(f"矩阵顺序漂移：期望 {uc}，实际 {row[0]}")

        actual_gates = tuple(part.strip() for part in row[1].split("/"))
        actual_gate_ids.extend(actual_gates)
        if actual_gates != gate_ids:
            errors.append(
                f"{uc} 机器 Gate 应为 {'/'.join(gate_ids)}，实际为 {row[1]}"
            )
        for gate_id in gate_ids:
            if f"| {gate_id} |" not in current_matrix:
                errors.append(f"当前代码矩阵缺少机器 Gate {gate_id}")

        links = LINK_RE.findall(row[2])
        expected_link = (
            f"../../docs/interaction-frame-v2-全量替换-按UC/{uc}/README.md"
        )
        if links != [expected_link]:
            errors.append(f"{uc} 当前源码合同链接漂移")
        elif not (FEATURE_ROOT / expected_link).resolve().is_file():
            errors.append(f"{uc} 当前源码合同不存在：{expected_link}")

        range_match = TASK_RANGE_RE.fullmatch(row[3])
        expected_last = first_task + 2
        if not range_match or tuple(map(int, range_match.groups())) != (
            first_task,
            expected_last,
        ):
            errors.append(
                f"{uc} 历史全局 Task 应为 T{first_task:03d}-T{expected_last:03d}"
            )

    expected_gate_ids = [
        gate_id for _, gate_ids, _ in EXPECTED_ROWS for gate_id in gate_ids
    ]
    if actual_gate_ids != expected_gate_ids:
        errors.append("矩阵必须按固定顺序精确覆盖 25 个机器 Gate")
    if {"G-03", "G-03a"} & set(actual_gate_ids):
        errors.append("G-03/G-03a 不得进入 018 当前源码矩阵")
    if len(actual_gate_ids) != 25:
        errors.append(f"机器 Gate 数量应为 25，实际为 {len(actual_gate_ids)}")


def main() -> int:
    """Run every documentation and historical task mapping check."""
    errors: list[str] = []
    verify_doc_shape(errors)
    verify_document_links(errors)
    verify_global_tasks(errors)
    verify_matrix(errors)

    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1

    print(
        "PASS: 8 个基础 UC x 8 + 10 个深挖 UC x 15 + "
        "G01 x 12（允许已存在的人工 handoff 扩展）当前文档；"
        "25 Gate；V1 切换期文档已下线；"
        "T021-T077 历史任务映射连续唯一"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
