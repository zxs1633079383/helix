#!/usr/bin/env python3
"""Fail-closed tests for the Helix Spec Kit Loop profile uninstaller."""

from __future__ import annotations

import importlib.util
import shutil
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "scripts" / "uninstall_profile.py"
SPEC = importlib.util.spec_from_file_location("uninstall_profile", SCRIPT)
assert SPEC and SPEC.loader
uninstall_profile = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(uninstall_profile)

PROJECT_ROOT = SCRIPT.parents[4]
PROFILE_FILES = (
    ".specify/memory/constitution.md",
    ".specify/templates/spec-template.md",
    ".specify/templates/plan-template.md",
    ".specify/templates/tasks-template.md",
)


class UninstallProfileTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for relative in PROFILE_FILES:
            destination = self.root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(PROJECT_ROOT / relative, destination)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_prepare_updates_restores_only_profile_blocks(self) -> None:
        updates = uninstall_profile.prepare_updates(self.root)
        self.assertEqual(set(updates), {self.root / relative for relative in PROFILE_FILES})
        self.assertNotIn(
            "**Loop Profile**: `helix-loop/v1`",
            updates[self.root / ".specify/templates/spec-template.md"],
        )
        self.assertNotIn(
            "## Loop Detail Contract",
            updates[self.root / ".specify/templates/plan-template.md"],
        )
        self.assertNotIn(
            "## Loop Task Contracts *(mandatory)*",
            updates[self.root / ".specify/templates/tasks-template.md"],
        )
        constitution = updates[self.root / ".specify/memory/constitution.md"]
        self.assertNotIn("### VI. Spec Kit Loop Admission", constitution)
        self.assertIn("**Version**: 1.0.0", constitution)

    def test_changed_managed_block_requires_human_decision(self) -> None:
        spec = self.root / ".specify/templates/spec-template.md"
        spec.write_text(
            spec.read_text(encoding="utf-8").replace(
                "## Loop Contract *(mandatory)*",
                "## Locally Changed Loop Contract",
                1,
            ),
            encoding="utf-8",
        )
        with self.assertRaises(uninstall_profile.RemovalBlocked):
            uninstall_profile.prepare_updates(self.root)


if __name__ == "__main__":
    unittest.main()
