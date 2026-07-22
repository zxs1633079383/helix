#!/usr/bin/env python3
"""Remove only the files and exact profile blocks installed by helix-loop."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


class RemovalBlocked(RuntimeError):
    """A profile-managed block was changed, missing, or duplicated."""


ROOT_FILES = (
    ".specify/memory/constitution.md",
    ".specify/templates/spec-template.md",
    ".specify/templates/plan-template.md",
    ".specify/templates/tasks-template.md",
)

SYNC_REPORT_CURRENT = """<!--
Sync Impact Report
- Version change: 1.0.0 -> 1.1.0
- Ratified principles: runtime-agnostic layering, deterministic effect flow,
  InteractionFrame V2 authority, evidence-first validation, minimal governed change,
  and Spec Kit loop-state admission.
- Templates updated: .specify/templates/spec-template.md,
  .specify/templates/plan-template.md, and .specify/templates/tasks-template.md.
- Added project extension: helix-loop supplies the persistent ledger, invalidation
  graph, validators, and negative tests that enforce these gates.
- Follow-up: existing feature bundles remain historical inputs. A bundle adopts this
  profile when its next Spec Kit command creates the required loop artifacts.
-->
"""

SYNC_REPORT_ORIGINAL = """<!--
Sync Impact Report
- Version change: template -> 1.0.0
- Ratified principles: runtime-agnostic layering, deterministic effect flow,
  InteractionFrame V2 authority, evidence-first validation, and minimal governed change.
- Templates checked: the generic plan/spec/tasks templates already require a
  constitution gate and task traceability; no template text needs a Helix-specific rewrite.
- Follow-up: every active feature plan must replace any template-constitution
  fallback with these executable gates before implementation starts.
-->
"""

CONSTITUTION_PRINCIPLE = """### VI. Spec Kit Loop Admission

Spec Kit remains the author of `spec.md`, `plan.md`, and `tasks.md`. Each feature
also has a source-discovery record, a persistent review ledger, and durable
verification evidence. The ledger is the sole workflow state: it records input
fingerprints, gate verdicts, the next allowed action, and bounded retry history.
The loop may advance only through a passing independent validator.

Reviews have one owner and one purpose: R1 validates behavior and completion,
R2 validates design, R3 validates task traceability, and R4 validates implemented
evidence. An unchanged fingerprint reuses its prior verdict. A changed source,
specification, plan, task list, or verification artifact invalidates only the
dependent downstream gates. The engine returns to the earliest invalid phase; it
does not rerun a whole feature review by habit.

Every feature records trigger, goal, completion, inputs and context, action,
validator, workflow state, boundaries, and any human gate exactly once in the
artifact that owns it. A business state machine belongs in the plan; workflow
state belongs only in the review ledger. A repeated failure with the same
task/validator signature may receive at most two automatic repair attempts.
The next recurrence enters `NeedsHuman`.

"""

CONSTITUTION_WORKFLOW_CURRENT = """Before implementation, the feature has a spec, plan, data model, wire contract,
task list, source discovery, valid R1-R3 ledger gates, and Constitution Check with
no unresolved template placeholders. Tasks are executed in dependency order, tests
precede their implementation, and completed tasks are marked in the task list only
after their stated evidence exists. Work that
"""

CONSTITUTION_WORKFLOW_ORIGINAL = """Before implementation, the feature has a spec, plan, data model, wire contract,
task list, and Constitution Check with no unresolved template placeholders. Tasks
are executed in dependency order, tests precede their implementation, and completed
tasks are marked in the task list only after their stated evidence exists. Work that
"""

SPEC_PROFILE_MARKER = "**Loop Profile**: `helix-loop/v1`\n\n"

SPEC_LOOP_CONTRACT = """## Loop Contract *(mandatory)*

> This is the external feature contract. Keep implementation mechanisms in
> `plan.md`; keep source citations in `discovery.md`.

### Trigger

[What event or precondition starts this feature path?]

### Goal

[What user- or system-visible result must this feature deliver?]

### Completion

[What observable evidence proves the goal is complete? Do not use an intermediate
transport acknowledgement as completion.]

### Boundaries

[What is explicitly out of scope? What retry, time, cost, or failure limit stops
automatic work?]

### Human Gate

[Which decisions require explicit approval? Write "None" only with a reason.]

"""

PLAN_LOOP_CONTRACT = """## Loop Detail Contract

> This is the technical counterpart of the external Loop Contract in `spec.md`.
> Source claims must link to `discovery.md`; each identifier is consumed by a task
> contract in `tasks.md`.

### Inputs and Context

- **I-001**: [Authoritative source, permitted reads, and relevant precondition]

### Action Chain

- **A-001**: [Input/state] -> [effect or change] -> [observable output]

### State Contract

- **S-001**: [State] -> [allowed transition] -> [terminal or next state]

Use `N/A: [reason]` only when the feature has no persistent business state.

### Validators

- **V-001**: [Independent command, rule, or inspection] proves [requirement].

"""

PLAN_TREE_ADDITIONS = (
    ("├── discovery.md         # Source facts and unresolved questions\n", "plan discovery artifact"),
    ("├── verification.md      # Repeatable acceptance procedure\n", "plan verification artifact"),
    ("├── review-ledger.json   # Loop state, fingerprints, and gate verdicts\n", "plan ledger artifact"),
    ("├── evidence/            # Hashed outputs from real verification runs\n", "plan evidence artifact"),
)

TASKS_FORMAT_ADDITION = """- Every task MUST have exactly one row in **Loop Task Contracts**. The row records
  requirement, plan item, change scope, precondition, validator, and done signal.
"""

TASKS_LOOP_CONTRACT = """## Loop Task Contracts *(mandatory)*

> This table is the compiler boundary between `plan.md` and implementation. It is
> not a second task list. Every checkbox task above must appear once, and no row may
> claim a task that does not exist.

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|------|-------------|-----------|--------------|---------------|-----------|-------------|
| T001 | FR-001 | A-001 | `path/to/file` | R1-R3 PASS | `command` | [observable result] |
"""


def project_root(script_path: Path, root_arg: str | None) -> Path:
    return Path(root_arg).resolve() if root_arg else script_path.resolve().parents[4]


def replace_exact(text: str, needle: str, replacement: str, label: str) -> str:
    count = text.count(needle)
    if count != 1:
        raise RemovalBlocked(f"{label}: expected one managed block, found {count}")
    return text.replace(needle, replacement, 1)


def remove_exact(text: str, needle: str, label: str) -> str:
    return replace_exact(text, needle, "", label)


def prepare_updates(root: Path) -> dict[Path, str]:
    """Return a fail-closed inverse patch without mutating project files."""

    paths = {relative: root / relative for relative in ROOT_FILES}
    missing = [relative for relative, path in paths.items() if not path.exists()]
    if missing:
        raise RemovalBlocked(f"missing profile-managed file(s): {', '.join(missing)}")

    constitution = paths[".specify/memory/constitution.md"].read_text(encoding="utf-8")
    constitution = replace_exact(
        constitution,
        SYNC_REPORT_CURRENT,
        SYNC_REPORT_ORIGINAL,
        "constitution sync impact report",
    )
    constitution = remove_exact(
        constitution,
        CONSTITUTION_PRINCIPLE,
        "constitution Spec Kit Loop Admission principle",
    )
    constitution = replace_exact(
        constitution,
        CONSTITUTION_WORKFLOW_CURRENT,
        CONSTITUTION_WORKFLOW_ORIGINAL,
        "constitution development workflow",
    )
    constitution = replace_exact(
        constitution,
        "**Version**: 1.1.0 | **Ratified**: 2026-07-21 | **Last Amended**: 2026-07-21",
        "**Version**: 1.0.0 | **Ratified**: 2026-07-21 | **Last Amended**: 2026-07-21",
        "constitution version",
    )

    spec = paths[".specify/templates/spec-template.md"].read_text(encoding="utf-8")
    spec = remove_exact(spec, SPEC_PROFILE_MARKER, "spec template loop profile marker")
    spec = remove_exact(spec, SPEC_LOOP_CONTRACT, "spec template loop contract")

    plan = paths[".specify/templates/plan-template.md"].read_text(encoding="utf-8")
    plan = remove_exact(plan, PLAN_LOOP_CONTRACT, "plan template loop detail contract")
    for needle, label in PLAN_TREE_ADDITIONS:
        plan = remove_exact(plan, needle, label)

    tasks = paths[".specify/templates/tasks-template.md"].read_text(encoding="utf-8")
    tasks = remove_exact(tasks, TASKS_FORMAT_ADDITION, "tasks template loop task rule")
    tasks = remove_exact(tasks, TASKS_LOOP_CONTRACT, "tasks template loop task contracts")

    return {
        paths[".specify/memory/constitution.md"]: constitution,
        paths[".specify/templates/spec-template.md"]: spec,
        paths[".specify/templates/plan-template.md"]: plan,
        paths[".specify/templates/tasks-template.md"]: tasks,
    }


def atomic_write(path: Path, content: str) -> None:
    temporary = path.with_name(f".{path.name}.helix-loop-remove.tmp")
    try:
        temporary.write_text(content, encoding="utf-8")
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def restore_files(updates: dict[Path, str]) -> None:
    originals = {path: path.read_text(encoding="utf-8") for path in updates}
    changed: list[Path] = []
    try:
        for path, content in updates.items():
            atomic_write(path, content)
            changed.append(path)
    except OSError:
        for path in reversed(changed):
            atomic_write(path, originals[path])
        raise


def remove_extension(root: Path) -> None:
    completed = subprocess.run(
        ["specify", "extension", "remove", "helix-loop", "--force"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "no CLI output"
        raise RuntimeError(f"Spec Kit extension removal failed: {detail}")


def emit(verdict: str, **payload: object) -> None:
    print(json.dumps({"verdict": verdict, **payload}, ensure_ascii=True, sort_keys=True))


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Safely uninstall only the Helix Spec Kit Loop profile infrastructure."
    )
    parser.add_argument("--root", help="Spec Kit project root; defaults to this script's project")
    parser.add_argument(
        "--apply",
        action="store_true",
        help="remove hooks/source and restore managed files after a successful dry-run",
    )
    args = parser.parse_args()
    root = project_root(Path(__file__), args.root)

    try:
        updates = prepare_updates(root)
    except RemovalBlocked as error:
        emit(
            "NEEDS_HUMAN",
            reason=str(error),
            preserved="specs/**",
            writes=[],
        )
        return 1

    managed_files = [str(path.relative_to(root)) for path in updates]
    if not args.apply:
        emit(
            "DRY_RUN",
            files=managed_files,
            removes=[
                ".specify/extensions/helix-loop",
                ".specify/extensions-src/helix-loop",
                "helix-loop hooks from .specify/extensions.yml",
            ],
            preserved="specs/**",
        )
        return 0

    try:
        remove_extension(root)
        restore_files(updates)
        source = root / ".specify" / "extensions-src" / "helix-loop"
        if source.exists():
            shutil.rmtree(source)
    except (OSError, RuntimeError) as error:
        emit(
            "PARTIAL",
            reason=str(error),
            preserved="specs/**",
            writes=managed_files,
        )
        return 1

    emit(
        "REMOVED",
        files=managed_files,
        removed=[
            ".specify/extensions/helix-loop",
            ".specify/extensions-src/helix-loop",
            "helix-loop hooks from .specify/extensions.yml",
        ],
        preserved="specs/**",
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
