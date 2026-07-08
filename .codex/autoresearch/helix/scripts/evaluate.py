#!/usr/bin/env python3
"""Evaluate the project-local AutoReSearch graduation bundle."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TOPIC = "helix"

REQUIRED_FILES = [
    "knowledge/helix_knowledge.md",
    "scripts/evaluate.py",
    "scripts/distill_project.py",
    "skills/helix-knowledge/SKILL.md",
    "agents/helix-researcher.md",
    "agents/helix-dev.md",
    "install.sh",
    "README.md",
]

REQUIRED_SECTIONS = [
    "## 0. Provenance And Scope",
    "## 1. Founder Thesis",
    "## 2. Core Maintainer Mental Model",
    "## 3. Architecture Map",
    "## 4. Domain And Runtime Concepts",
    "## 5. Invariants And Design Boundaries",
    "## 6. Development And Verification Playbook",
    "## 7. Contribution And Product Opportunities",
    "## 8. Production Readiness Radar",
    "## 9. Agent Rules",
    "## 10. Open Questions",
]

FORBIDDEN_PATTERNS = [
    re.compile("/" + "System/Volumes/Data/"),
    re.compile("/" + r"Users/[^/\s]+/"),
    re.compile("/" + r"workspace/(rust|golang|java|c|pd)/"),
    re.compile("~/" + ".Codex"),
    re.compile("~/" + ".codex/skills"),
    re.compile("~/" + ".codex/agents"),
]


def fail(message: str) -> None:
    print(f"FAIL: {message}")
    sys.exit(1)


def read_rel(path: str) -> str:
    target = ROOT / path
    if not target.exists():
        fail(f"missing required file: {path}")
    return target.read_text(encoding="utf-8")


def check_required_files() -> None:
    missing = [path for path in REQUIRED_FILES if not (ROOT / path).exists()]
    if missing:
        fail("missing required files: " + ", ".join(missing))


def check_knowledge_sections() -> None:
    text = read_rel("knowledge/helix_knowledge.md")
    missing = [section for section in REQUIRED_SECTIONS if section not in text]
    if missing:
        fail("knowledge missing sections: " + ", ".join(missing))


def check_no_forbidden_absolute_refs() -> None:
    offenders: list[str] = []
    for path in ROOT.rglob("*"):
        if path.is_dir() or path.name == "__pycache__":
            continue
        rel = path.relative_to(ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        for pattern in FORBIDDEN_PATTERNS:
            if pattern.search(text):
                offenders.append(f"{rel}: {pattern.pattern}")
    if offenders:
        fail("forbidden absolute/global references found: " + "; ".join(offenders))


def check_relative_deep_links() -> None:
    skill = read_rel("skills/helix-knowledge/SKILL.md")
    researcher = read_rel("agents/helix-researcher.md")
    dev = read_rel("agents/helix-dev.md")
    if "../../knowledge/helix_knowledge.md" not in skill:
        fail("skill missing relative knowledge pointer")
    for needle in [
        "../knowledge/helix_knowledge.md",
        "../scripts/evaluate.py",
        "../scripts/distill_project.py",
    ]:
        if needle not in researcher:
            fail(f"researcher missing relative pointer: {needle}")
    if "../knowledge/helix_knowledge.md" not in dev:
        fail("dev agent missing relative knowledge pointer")
    if "python3 scripts/evaluate.py" not in dev:
        fail("dev agent missing bundle-root evaluator command")


def report_coverage() -> None:
    knowledge = read_rel("knowledge/helix_knowledge.md")
    words = re.findall(r"[\w:/.-]+", knowledge)
    print(f"Bundle root: {ROOT}")
    print(f"Required files: {len(REQUIRED_FILES)}/{len(REQUIRED_FILES)}")
    print(f"Knowledge sections: {len(REQUIRED_SECTIONS)}/{len(REQUIRED_SECTIONS)}")
    print(f"Knowledge tokens-ish: {len(words)}")
    print("PASS: helix AutoReSearch graduation bundle structure is valid")


def main() -> None:
    check_required_files()
    check_knowledge_sections()
    check_no_forbidden_absolute_refs()
    check_relative_deep_links()
    report_coverage()


if __name__ == "__main__":
    main()
