#!/usr/bin/env python3
"""Persistent, deterministic admission gates for project-local Spec Kit features."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


SCHEMA_VERSION = "1.0"
GATE_ORDER = ("r1_spec", "r2_plan", "r3_tasks", "r4_evidence")
GATE_PHASE = {
    "r1_spec": "specifying",
    "r2_plan": "planning",
    "r3_tasks": "tasking",
    "r4_evidence": "verifying",
}
GATE_FILES = {
    "r1_spec": ("discovery.md", "spec.md"),
    "r2_plan": ("discovery.md", "spec.md", "plan.md"),
    "r3_tasks": ("discovery.md", "spec.md", "plan.md", "tasks.md"),
    "r4_evidence": (
        "discovery.md",
        "spec.md",
        "plan.md",
        "tasks.md",
        "verification.md",
        "evidence/evidence.json",
    ),
}
SPEC_SECTIONS = (
    "## Loop Contract",
    "### Trigger",
    "### Goal",
    "### Completion",
    "### Boundaries",
    "### Human Gate",
)
PLAN_SECTIONS = (
    "## Loop Detail Contract",
    "### Inputs and Context",
    "### Action Chain",
    "### State Contract",
    "### Validators",
)
TASK_TABLE_HEADER = (
    "| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |"
)
PLACEHOLDER = re.compile(
    r"\[(?:FEATURE|DATE|Describe|Brief|Goal|What|Replace|NEEDS CLARIFICATION|"
    r"Authoritative source|Input/state|State|Independent command|observable result)",
    re.IGNORECASE,
)
TASK_ID = re.compile(r"^- \[[ xX]\] (T\d{3,})\b", re.MULTILINE)
CONTRACT_TASK_ID = re.compile(r"^\|\s*(T\d{3,})\s*\|", re.MULTILINE)
FACT_LINE = re.compile(r"^- \*\*F-\d+\*\*: .+\| \*\*Evidence\*\*: `[^`]+:\d+`", re.MULTILINE)


def utc_now() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def project_root(script_path: Path, root_arg: str | None) -> Path:
    if root_arg:
        return Path(root_arg).resolve()
    return script_path.resolve().parents[4]


def read_json(path: Path, default: Any = None) -> Any:
    if not path.exists():
        return default
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def git_value(root: Path, *args: str) -> str:
    try:
        return subprocess.check_output(
            ["git", *args],
            cwd=root,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unavailable"


def resolve_feature(root: Path, feature_arg: str | None) -> Path:
    if feature_arg:
        feature = Path(feature_arg)
        return feature if feature.is_absolute() else root / feature
    config = read_json(root / ".specify" / "feature.json")
    if not isinstance(config, dict) or not isinstance(config.get("feature_directory"), str):
        raise ValueError("missing .specify/feature.json feature_directory; pass --feature explicitly")
    return root / config["feature_directory"]


def relative(root: Path, path: Path) -> str:
    return str(path.resolve().relative_to(root.resolve()))


def artifact_paths(root: Path, feature: Path, gate: str) -> list[Path]:
    paths = [feature / relative_path for relative_path in GATE_FILES[gate]]
    return [root / ".specify" / "memory" / "constitution.md", *paths]


def fingerprint(root: Path, feature: Path, gate: str) -> str:
    digest = hashlib.sha256()
    for path in artifact_paths(root, feature, gate):
        digest.update(relative(root, path).encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes() if path.exists() else b"<MISSING>")
        digest.update(b"\0")
    return digest.hexdigest()


def new_ledger(root: Path, feature: Path) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "feature_directory": relative(root, feature),
        "phase": "specifying",
        "event_seq": 0,
        "next_action": "complete_spec_and_discovery",
        "budget": {"max_same_failure_attempts": 2},
        "failure_attempts": {},
        "gates": {
            gate: {"status": "pending", "fingerprint": None, "checked_at": None, "issues": []}
            for gate in GATE_ORDER
        },
        "convergence": {"tasks_fingerprint": None, "reason": None},
        "events": [],
    }


def ledger_path(feature: Path) -> Path:
    return feature / "review-ledger.json"


def profile_enabled(feature: Path) -> bool:
    spec = feature / "spec.md"
    return spec.exists() and "**Loop Profile**: `helix-loop/v1`" in spec.read_text(encoding="utf-8")


def load_ledger(root: Path, feature: Path) -> dict[str, Any]:
    path = ledger_path(feature)
    if not path.exists():
        return new_ledger(root, feature)
    value = read_json(path)
    if not isinstance(value, dict) or value.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(f"invalid loop ledger: {relative(root, path)}")
    return value


def save_ledger(feature: Path, ledger: dict[str, Any]) -> None:
    write_json(ledger_path(feature), ledger)


def event(ledger: dict[str, Any], kind: str, detail: str) -> None:
    ledger["event_seq"] += 1
    ledger["events"].append(
        {"seq": ledger["event_seq"], "at": utc_now(), "kind": kind, "detail": detail}
    )


def set_phase(ledger: dict[str, Any], phase: str, next_action: str, detail: str) -> None:
    previous = ledger.get("phase")
    ledger["phase"] = phase
    ledger["next_action"] = next_action
    event(ledger, "phase", f"{previous}->{phase}: {detail}")


def source_discovery_template(root: Path, feature: Path) -> str:
    return f"""# Source Discovery

**Feature**: `{relative(root, feature)}`
**Source Lock**: `{git_value(root, "rev-parse", "HEAD")}`
**Worktree State**: `{git_value(root, "status", "--porcelain") or "clean"}`

## Facts *(mandatory)*

- **F-001**: [Replace with a source-grounded fact] | **Evidence**: `path/to/file:1`

## Unknowns

- **U-001**: [Replace with an unresolved question or write None.] | **Resolution**: [clarify, human gate, or source]

## Decisions

- **D-001**: [Replace with a decision backed by facts, or write None.]
"""


def verification_template() -> str:
    return """# Verification Procedure

## Required Runs

- **V-001**: [Command and expected falsifiable result]

## Evidence Rules

Store raw command output beneath `evidence/`. Register each run with
`loop_engine.py record-evidence`; do not replace a failed output with prose.
"""


def ensure_feature_artifacts(root: Path, feature: Path) -> list[str]:
    feature.mkdir(parents=True, exist_ok=True)
    created: list[str] = []
    discovery = feature / "discovery.md"
    if not discovery.exists():
        discovery.write_text(source_discovery_template(root, feature), encoding="utf-8")
        created.append(relative(root, discovery))
    verification = feature / "verification.md"
    if not verification.exists():
        verification.write_text(verification_template(), encoding="utf-8")
        created.append(relative(root, verification))
    (feature / "evidence").mkdir(exist_ok=True)
    if not ledger_path(feature).exists():
        save_ledger(feature, new_ledger(root, feature))
        created.append(relative(root, ledger_path(feature)))
    return created


def section_has_content(text: str, heading: str) -> bool:
    match = re.search(
        rf"^{re.escape(heading)}(?:\s+\*\(mandatory\)\*)?\s*$",
        text,
        re.MULTILINE,
    )
    if not match:
        return False
    level = len(heading) - len(heading.lstrip("#"))
    rest = text[match.end() :]
    next_heading = re.search(rf"^#{{1,{level}}}\s+", rest, re.MULTILINE)
    body = rest[: next_heading.start() if next_heading else len(rest)]
    body = re.sub(r"<!--.*?-->", "", body, flags=re.DOTALL)
    return bool(body.strip()) and not PLACEHOLDER.search(body)


def validate_discovery(feature: Path) -> list[str]:
    path = feature / "discovery.md"
    if not path.exists():
        return ["missing discovery.md"]
    text = path.read_text(encoding="utf-8")
    issues: list[str] = []
    if not re.search(r"^\*\*Source Lock\*\*: `[^`]+`$", text, re.MULTILINE):
        issues.append("discovery.md has no source lock")
    if not FACT_LINE.search(text):
        issues.append("discovery.md needs at least one F-xxx fact with path:line evidence")
    if PLACEHOLDER.search(text):
        issues.append("discovery.md still contains a template placeholder")
    return issues


def validate_spec(feature: Path) -> list[str]:
    path = feature / "spec.md"
    if not path.exists():
        return ["missing spec.md"]
    text = path.read_text(encoding="utf-8")
    issues = [
        f"spec.md missing completed section {heading}"
        for heading in SPEC_SECTIONS
        if not section_has_content(text, heading)
    ]
    if "[NEEDS CLARIFICATION" in text:
        issues.append("spec.md has unresolved NEEDS CLARIFICATION")
    return issues


def validate_plan(feature: Path) -> list[str]:
    path = feature / "plan.md"
    if not path.exists():
        return ["missing plan.md"]
    text = path.read_text(encoding="utf-8")
    issues = [
        f"plan.md missing completed section {heading}"
        for heading in PLAN_SECTIONS
        if not section_has_content(text, heading)
    ]
    if not re.search(r"\bI-\d{3}\b", text):
        issues.append("plan.md has no I-xxx context item")
    if not re.search(r"\bA-\d{3}\b", text):
        issues.append("plan.md has no A-xxx action item")
    if not (re.search(r"\bS-\d{3}\b", text) or re.search(r"^N/A:", text, re.MULTILINE)):
        issues.append("plan.md has no S-xxx state item or N/A rationale")
    if not re.search(r"\bV-\d{3}\b", text):
        issues.append("plan.md has no V-xxx validator item")
    return issues


def validate_tasks(feature: Path) -> list[str]:
    path = feature / "tasks.md"
    if not path.exists():
        return ["missing tasks.md"]
    text = path.read_text(encoding="utf-8")
    task_ids = set(TASK_ID.findall(text))
    contract_ids = set(CONTRACT_TASK_ID.findall(text))
    issues: list[str] = []
    if not task_ids:
        issues.append("tasks.md has no checkbox task IDs")
    if TASK_TABLE_HEADER not in text:
        issues.append("tasks.md is missing the Loop Task Contracts table header")
    missing = sorted(task_ids - contract_ids)
    extra = sorted(contract_ids - task_ids)
    if missing:
        issues.append(f"Loop Task Contracts missing task rows: {', '.join(missing)}")
    if extra:
        issues.append(f"Loop Task Contracts contains unknown task rows: {', '.join(extra)}")
    if re.search(r"^\|\s*T\d{3,}\s*\|[^\n]*\[[^\]]+\]", text, re.MULTILINE):
        issues.append("Loop Task Contracts contains an unresolved placeholder")
    return issues


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def evidence_manifest_path(feature: Path) -> Path:
    return feature / "evidence" / "evidence.json"


def validate_evidence(root: Path, feature: Path) -> list[str]:
    path = evidence_manifest_path(feature)
    if not path.exists():
        return ["missing evidence/evidence.json"]
    try:
        manifest = read_json(path)
    except json.JSONDecodeError:
        return ["evidence/evidence.json is invalid JSON"]
    if not isinstance(manifest, dict) or manifest.get("schema_version") != SCHEMA_VERSION:
        return ["evidence manifest has an unsupported schema version"]
    runs = manifest.get("runs")
    if not isinstance(runs, list) or not runs:
        return ["evidence manifest needs at least one run"]
    issues: list[str] = []
    has_pass = False
    evidence_root = (feature / "evidence").resolve()
    for run in runs:
        if not isinstance(run, dict):
            issues.append("evidence manifest contains a non-object run")
            continue
        required = ("id", "command", "started_at", "ended_at", "result", "files")
        missing = [name for name in required if not run.get(name)]
        if missing:
            issues.append(f"evidence run missing fields: {', '.join(missing)}")
            continue
        if run["result"] == "PASS":
            has_pass = True
        elif run["result"] not in ("FAIL", "BLOCKED"):
            issues.append(f"evidence run {run['id']} has invalid result {run['result']}")
        if not isinstance(run["files"], list) or not run["files"]:
            issues.append(f"evidence run {run['id']} has no files")
            continue
        for file_record in run["files"]:
            if not isinstance(file_record, dict) or not file_record.get("path") or not file_record.get("sha256"):
                issues.append(f"evidence run {run['id']} has an invalid file record")
                continue
            evidence_file = root / file_record["path"]
            try:
                evidence_file.resolve().relative_to(evidence_root)
            except ValueError:
                issues.append(f"evidence file escapes feature evidence directory: {file_record['path']}")
                continue
            if not evidence_file.exists():
                issues.append(f"evidence file is missing: {file_record['path']}")
            elif sha256(evidence_file) != file_record["sha256"]:
                issues.append(f"evidence file digest mismatch: {file_record['path']}")
    if not has_pass:
        issues.append("evidence manifest has no PASS run")
    return issues


def current_fingerprints(root: Path, feature: Path) -> dict[str, str]:
    return {gate: fingerprint(root, feature, gate) for gate in GATE_ORDER}


def invalidate_changed(root: Path, feature: Path, ledger: dict[str, Any]) -> str | None:
    current = current_fingerprints(root, feature)
    earliest: str | None = None
    for index, gate in enumerate(GATE_ORDER):
        gate_state = ledger["gates"][gate]
        if gate_state["status"] == "pass" and gate_state["fingerprint"] != current[gate]:
            for downstream in GATE_ORDER[index:]:
                downstream_state = ledger["gates"][downstream]
                if downstream_state["status"] == "pass":
                    downstream_state["status"] = "invalid"
                    downstream_state["issues"] = [f"invalidated by changed input for {gate}"]
            earliest = earliest or gate
    if earliest:
        phase = GATE_PHASE[earliest]
        set_phase(ledger, phase, f"rerun_{earliest}", f"input fingerprint changed for {earliest}")
    return earliest


def gate_is_current(root: Path, feature: Path, ledger: dict[str, Any], gate: str) -> bool:
    state = ledger["gates"][gate]
    return state["status"] == "pass" and state["fingerprint"] == fingerprint(root, feature, gate)


def record_failure(ledger: dict[str, Any], stage: str, issues: list[str]) -> bool:
    signature = hashlib.sha256((stage + "\n" + "\n".join(issues)).encode("utf-8")).hexdigest()[:16]
    attempts = ledger["failure_attempts"].get(signature, 0) + 1
    ledger["failure_attempts"][signature] = attempts
    event(ledger, "failure", f"{stage}:{signature}: attempt {attempts}")
    return attempts > ledger["budget"]["max_same_failure_attempts"]


def emit(verdict: str, ledger: dict[str, Any], **extra: Any) -> None:
    payload = {
        "verdict": verdict,
        "phase": ledger["phase"],
        "next_action": ledger["next_action"],
        "event_seq": ledger["event_seq"],
        **extra,
    }
    print(json.dumps(payload, sort_keys=True, ensure_ascii=True))


def block(feature: Path, ledger: dict[str, Any], stage: str, issues: list[str]) -> int:
    ledger["gates"][stage]["status"] = "blocked"
    ledger["gates"][stage]["issues"] = issues
    needs_human = record_failure(ledger, stage, issues)
    if needs_human:
        set_phase(ledger, "needs_human", "human_decision", f"{stage} exceeded automatic retry budget")
        verdict = "NEEDS_HUMAN"
    else:
        set_phase(ledger, GATE_PHASE.get(stage, ledger["phase"]), f"repair_{stage}", "; ".join(issues))
        verdict = "BLOCKED"
    save_ledger(feature, ledger)
    emit(verdict, ledger, stage=stage, issues=issues)
    return 1


def pass_gate(root: Path, feature: Path, ledger: dict[str, Any], gate: str, phase: str, next_action: str) -> int:
    issues = {
        "r1_spec": validate_discovery(feature) + validate_spec(feature),
        "r2_plan": validate_plan(feature),
        "r3_tasks": validate_tasks(feature),
        "r4_evidence": validate_evidence(root, feature),
    }[gate]
    if issues:
        return block(feature, ledger, gate, issues)
    state = ledger["gates"][gate]
    state.update(
        {
            "status": "pass",
            "fingerprint": fingerprint(root, feature, gate),
            "checked_at": utc_now(),
            "issues": [],
        }
    )
    set_phase(ledger, phase, next_action, f"{gate} passed")
    save_ledger(feature, ledger)
    emit("PASS", ledger, gate=gate)
    return 0


def admit(root: Path, feature: Path, ledger: dict[str, Any]) -> int:
    invalidate_changed(root, feature, ledger)
    missing = [gate for gate in GATE_ORDER[:3] if not gate_is_current(root, feature, ledger, gate)]
    if missing:
        return block(feature, ledger, "r3_tasks", [f"implementation requires current PASS: {', '.join(missing)}"])
    set_phase(ledger, "implementing", "implement_tasks", "R1-R3 admitted implementation")
    save_ledger(feature, ledger)
    emit("IMPLEMENTATION_ADMITTED", ledger)
    return 0


def begin_converge(root: Path, feature: Path, ledger: dict[str, Any]) -> int:
    ledger["convergence"] = {
        "tasks_fingerprint": fingerprint(root, feature, "r3_tasks"),
        "reason": ledger["gates"]["r4_evidence"]["issues"] or "evidence accepted",
    }
    set_phase(ledger, "converging", "run_speckit_converge", "targeted convergence started")
    save_ledger(feature, ledger)
    emit("PASS", ledger, gate="converge")
    return 0


def finish_converge(root: Path, feature: Path, ledger: dict[str, Any]) -> int:
    previous = ledger["convergence"].get("tasks_fingerprint")
    current = fingerprint(root, feature, "r3_tasks")
    if previous != current:
        for gate in ("r3_tasks", "r4_evidence"):
            ledger["gates"][gate]["status"] = "invalid"
            ledger["gates"][gate]["issues"] = ["tasks changed during convergence"]
        set_phase(ledger, "tasking", "run_speckit_tasks", "convergence appended or changed tasks")
        save_ledger(feature, ledger)
        emit("TASKS_APPENDED", ledger)
        return 0
    if gate_is_current(root, feature, ledger, "r4_evidence"):
        set_phase(ledger, "closed", "none", "convergence found no additional task")
        save_ledger(feature, ledger)
        emit("CLOSED", ledger)
        return 0
    return block(feature, ledger, "r4_evidence", ["convergence changed no task and evidence is not PASS"])


def advance(root: Path, feature: Path) -> int:
    if not profile_enabled(feature):
        print(
            json.dumps(
                {
                    "verdict": "PROFILE_NOT_ENABLED",
                    "phase": "unmanaged",
                    "next_action": "use_standard_speckit_or_add_helix-loop/v1",
                },
                sort_keys=True,
                ensure_ascii=True,
            )
        )
        return 0
    ensure_feature_artifacts(root, feature)
    ledger = load_ledger(root, feature)
    invalidate_changed(root, feature, ledger)
    phase = ledger["phase"]
    if phase == "needs_human":
        save_ledger(feature, ledger)
        emit("NEEDS_HUMAN", ledger, issues=["recorded retry budget exceeded"])
        return 1
    if not gate_is_current(root, feature, ledger, "r1_spec"):
        return pass_gate(root, feature, ledger, "r1_spec", "planning", "run_speckit_plan")
    if not (feature / "plan.md").exists():
        save_ledger(feature, ledger)
        emit("READY_FOR_PLAN", ledger)
        return 0
    if not gate_is_current(root, feature, ledger, "r2_plan"):
        return pass_gate(root, feature, ledger, "r2_plan", "tasking", "run_speckit_tasks")
    if not (feature / "tasks.md").exists():
        save_ledger(feature, ledger)
        emit("READY_FOR_TASKS", ledger)
        return 0
    if not gate_is_current(root, feature, ledger, "r3_tasks"):
        return pass_gate(root, feature, ledger, "r3_tasks", "ready", "admit_implementation")
    if phase in ("ready", "tasking", "planning"):
        return admit(root, feature, ledger)
    if phase == "implementing":
        return pass_gate(root, feature, ledger, "r4_evidence", "verifying", "run_speckit_converge_or_close")
    if phase == "verifying":
        r4_status = ledger["gates"]["r4_evidence"]["status"]
        if gate_is_current(root, feature, ledger, "r4_evidence") or (
            r4_status == "blocked" and evidence_manifest_path(feature).exists()
        ):
            return begin_converge(root, feature, ledger)
        return pass_gate(root, feature, ledger, "r4_evidence", "verifying", "run_speckit_converge_or_close")
    if phase == "converging":
        return finish_converge(root, feature, ledger)
    if phase == "closed":
        save_ledger(feature, ledger)
        emit("CLOSED", ledger)
        return 0
    return block(feature, ledger, "r4_evidence", [f"unknown loop phase {phase}"])


def record_evidence(root: Path, feature: Path, args: argparse.Namespace) -> int:
    if not profile_enabled(feature):
        raise ValueError("feature does not declare **Loop Profile**: `helix-loop/v1`")
    file_path = Path(args.file)
    evidence_file = file_path if file_path.is_absolute() else root / file_path
    evidence_root = (feature / "evidence").resolve()
    try:
        evidence_file.resolve().relative_to(evidence_root)
    except ValueError:
        raise ValueError("--file must be inside the active feature evidence directory")
    if not evidence_file.exists():
        raise ValueError(f"evidence file does not exist: {relative(root, evidence_file)}")
    manifest_path = evidence_manifest_path(feature)
    manifest = read_json(manifest_path, {"schema_version": SCHEMA_VERSION, "runs": []})
    if not isinstance(manifest, dict) or manifest.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("invalid existing evidence manifest")
    run = {
        "id": args.id,
        "command": args.command,
        "started_at": args.started_at,
        "ended_at": args.ended_at,
        "result": args.result,
        "files": [{"path": relative(root, evidence_file), "sha256": sha256(evidence_file)}],
    }
    manifest["runs"] = [existing for existing in manifest.get("runs", []) if existing.get("id") != args.id]
    manifest["runs"].append(run)
    write_json(manifest_path, manifest)
    ledger = load_ledger(root, feature)
    set_phase(ledger, "implementing", "validate_evidence", f"recorded evidence {args.id}")
    save_ledger(feature, ledger)
    emit("PASS", ledger, evidence_id=args.id)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", help="Project root; defaults to the installed extension location")
    parser.add_argument("--feature", help="Feature directory relative to project root")
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("advance")
    subparsers.add_parser("status")
    evidence = subparsers.add_parser("record-evidence")
    evidence.add_argument("--id", required=True)
    evidence.add_argument("--command", required=True)
    evidence.add_argument("--started-at", required=True)
    evidence.add_argument("--ended-at", required=True)
    evidence.add_argument("--result", required=True, choices=("PASS", "FAIL", "BLOCKED"))
    evidence.add_argument("--file", required=True)
    args = parser.parse_args()
    root = project_root(Path(__file__), args.root)
    feature = resolve_feature(root, args.feature)
    try:
        if args.command == "advance":
            return advance(root, feature)
        if args.command == "status":
            if not profile_enabled(feature):
                print(
                    json.dumps(
                        {
                            "verdict": "PROFILE_NOT_ENABLED",
                            "phase": "unmanaged",
                            "next_action": "use_standard_speckit_or_add_helix-loop/v1",
                        },
                        sort_keys=True,
                        ensure_ascii=True,
                    )
                )
                return 0
            ensure_feature_artifacts(root, feature)
            ledger = load_ledger(root, feature)
            invalidate_changed(root, feature, ledger)
            save_ledger(feature, ledger)
            emit("STATUS", ledger, gates=ledger["gates"])
            return 0
        return record_evidence(root, feature, args)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(json.dumps({"verdict": "BLOCKED", "error": str(error)}, sort_keys=True), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
