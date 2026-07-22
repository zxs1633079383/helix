#!/usr/bin/env python3
"""Negative and transition tests for the project-local Spec Kit loop extension."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "scripts" / "loop_engine.py"
SPEC = importlib.util.spec_from_file_location("loop_engine", SCRIPT)
assert SPEC and SPEC.loader
loop_engine = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(loop_engine)


VALID_SPEC = """# Feature

**Loop Profile**: `helix-loop/v1`

## User Scenarios & Testing

### User Story 1

The user completes a visible action.

## Requirements

- **FR-001**: The system MUST complete the action.

## Success Criteria

- **SC-001**: The visible result is observed.

## Loop Contract *(mandatory)*

### Trigger

The user invokes the action.

### Goal

The user receives the result.

### Completion

A verifier observes the result.

### Boundaries

Only this feature is changed; automatic repair stops after two equal failures.

### Human Gate

None: no irreversible decision is involved.
"""

VALID_PLAN = """# Plan

## Constitution Check

PASS.

## Loop Detail Contract

### Inputs and Context

- **I-001**: Existing source facts are read-only inputs.

### Action Chain

- **A-001**: Command -> update -> observable result.

### State Contract

- **S-001**: Pending -> Completed.

### Validators

- **V-001**: `python3 check.py` proves FR-001.
"""

VALID_TASKS = """# Tasks

- [ ] T001 [US1] Implement the action in `src/example.rs`

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|------|-------------|-----------|--------------|---------------|-----------|-------------|
| T001 | FR-001 | A-001 | `src/example.rs` | R1-R3 PASS | `python3 check.py` | visible result |
"""


class LoopEngineTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / ".specify" / "memory").mkdir(parents=True)
        (self.root / ".specify" / "memory" / "constitution.md").write_text(
            "# Constitution\n",
            encoding="utf-8",
        )
        self.feature = self.root / "specs" / "001-demo"
        self.feature.mkdir(parents=True)
        (self.root / ".specify" / "feature.json").write_text(
            json.dumps({"feature_directory": "specs/001-demo"}),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def advance(self) -> int:
        return loop_engine.advance(self.root, self.feature)

    def make_discovery(self) -> None:
        (self.feature / "discovery.md").write_text(
            """# Source Discovery

**Source Lock**: `abc123`

## Facts

- **F-001**: The existing path owns the result. | **Evidence**: `src/example.rs:10`
""",
            encoding="utf-8",
        )

    def test_missing_source_fact_blocks_spec_admission(self) -> None:
        (self.feature / "spec.md").write_text(VALID_SPEC, encoding="utf-8")
        self.assertEqual(self.advance(), 1)
        ledger = json.loads((self.feature / "review-ledger.json").read_text(encoding="utf-8"))
        self.assertEqual(ledger["gates"]["r1_spec"]["status"], "blocked")
        self.assertEqual(ledger["phase"], "specifying")

    def test_legacy_feature_is_not_retroactively_blocked(self) -> None:
        (self.feature / "spec.md").write_text("# Legacy Feature\n", encoding="utf-8")
        self.assertEqual(self.advance(), 0)
        self.assertFalse((self.feature / "review-ledger.json").exists())

    def test_plan_change_invalidates_only_downstream_gates(self) -> None:
        (self.feature / "spec.md").write_text(VALID_SPEC, encoding="utf-8")
        self.make_discovery()
        self.assertEqual(self.advance(), 0)
        (self.feature / "plan.md").write_text(VALID_PLAN, encoding="utf-8")
        self.assertEqual(self.advance(), 0)
        (self.feature / "tasks.md").write_text(VALID_TASKS, encoding="utf-8")
        self.assertEqual(self.advance(), 0)
        self.assertEqual(self.advance(), 0)
        (self.feature / "plan.md").write_text(
            VALID_PLAN + "\nChanged design detail.\n",
            encoding="utf-8",
        )
        self.assertEqual(self.advance(), 0)
        ledger = json.loads((self.feature / "review-ledger.json").read_text(encoding="utf-8"))
        self.assertEqual(ledger["gates"]["r1_spec"]["status"], "pass")
        self.assertEqual(ledger["gates"]["r2_plan"]["status"], "pass")
        self.assertEqual(ledger["gates"]["r3_tasks"]["status"], "invalid")
        self.assertEqual(ledger["phase"], "tasking")

    def test_task_without_contract_row_blocks(self) -> None:
        (self.feature / "spec.md").write_text(VALID_SPEC, encoding="utf-8")
        self.make_discovery()
        self.assertEqual(self.advance(), 0)
        (self.feature / "plan.md").write_text(VALID_PLAN, encoding="utf-8")
        self.assertEqual(self.advance(), 0)
        (self.feature / "tasks.md").write_text(
            "# Tasks\n\n- [ ] T001 [US1] Implement it\n\n## Loop Task Contracts\n",
            encoding="utf-8",
        )
        self.assertEqual(self.advance(), 1)

    def test_evidence_digest_mismatch_blocks_completion(self) -> None:
        (self.feature / "spec.md").write_text(VALID_SPEC, encoding="utf-8")
        self.make_discovery()
        self.assertEqual(self.advance(), 0)
        (self.feature / "plan.md").write_text(VALID_PLAN, encoding="utf-8")
        self.assertEqual(self.advance(), 0)
        (self.feature / "tasks.md").write_text(VALID_TASKS, encoding="utf-8")
        self.assertEqual(self.advance(), 0)
        self.assertEqual(self.advance(), 0)
        evidence_dir = self.feature / "evidence"
        evidence_dir.mkdir(exist_ok=True)
        output = evidence_dir / "run.txt"
        output.write_text("PASS\n", encoding="utf-8")

        class Args:
            id = "V-001"
            command = "python3 check.py"
            started_at = "2026-07-21T00:00:00Z"
            ended_at = "2026-07-21T00:00:01Z"
            result = "PASS"
            file = "specs/001-demo/evidence/run.txt"

        self.assertEqual(loop_engine.record_evidence(self.root, self.feature, Args()), 0)
        self.assertEqual(self.advance(), 0)
        self.assertEqual(self.advance(), 0)
        self.assertEqual(self.advance(), 0)
        ledger = json.loads((self.feature / "review-ledger.json").read_text(encoding="utf-8"))
        self.assertEqual(ledger["phase"], "closed")
        manifest_path = evidence_dir / "evidence.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["runs"][0]["files"][0]["sha256"] = "0" * 64
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        self.assertEqual(self.advance(), 1)


if __name__ == "__main__":
    unittest.main()
