#!/usr/bin/env python3
"""Read-only inventory helper for refreshing the Helix knowledge bundle."""

from __future__ import annotations

import argparse
import subprocess
from pathlib import Path


def run(project: Path, *cmd: str) -> str:
    try:
        result = subprocess.run(
            cmd,
            cwd=project,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except FileNotFoundError:
        return ""
    return result.stdout.strip()


def list_paths(project: Path, pattern: str, max_items: int = 80) -> list[str]:
    return sorted(
        path.relative_to(project).as_posix()
        for path in project.glob(pattern)
        if path.is_file()
    )[:max_items]


def main() -> None:
    parser = argparse.ArgumentParser(description="Print read-only repo inventory signals.")
    parser.add_argument("--project", default="../../..", help="Project root relative to bundle root.")
    args = parser.parse_args()

    project = Path(args.project).resolve()
    print(f"# Project inventory: {project.name}")
    print()

    print("## Git")
    print(f"- branch: {run(project, 'git', 'rev-parse', '--abbrev-ref', 'HEAD') or 'unknown'}")
    print(f"- head: {run(project, 'git', 'rev-parse', '--short', 'HEAD') or 'unknown'}")
    status = run(project, "git", "status", "--short")
    print(f"- dirty entries: {len(status.splitlines()) if status else 0}")
    print()

    print("## Workspace crates")
    cargo = project / "Cargo.toml"
    if cargo.exists():
        in_members = False
        for line in cargo.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if stripped == "members = [":
                in_members = True
                continue
            if in_members and stripped == "]":
                break
            if in_members and stripped.startswith('"'):
                print(f"- {stripped.strip(',').strip(chr(34))}")
    print()

    print("## Key docs")
    for path in [
        "README.md",
        "docs/GOAL.md",
        "docs/ROADMAP.md",
        "docs/harness/README.md",
        "docs/harness/OPERATING.md",
        "docs/harness/decisions.md",
        "docs/COMMIT_CONVENTION.md",
    ]:
        marker = "yes" if (project / path).exists() else "missing"
        print(f"- {path}: {marker}")
    print()

    print("## Harness cards")
    for path in list_paths(project, "docs/harness/HX-C*.md", max_items=40):
        print(f"- {path}")
    print()

    print("## Scripts")
    for path in list_paths(project, "scripts/*.sh", max_items=80):
        print(f"- {path}")


if __name__ == "__main__":
    main()
