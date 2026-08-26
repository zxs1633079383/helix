"""一键发布脚本的纯版本与 manifest 改写回归。"""

from __future__ import annotations

import importlib.util
import sys
import tomllib
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).resolve().parents[1] / "publish_crates.py"
SPEC = importlib.util.spec_from_file_location("publish_crates", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"无法加载 {SCRIPT_PATH}")
PUBLISH_CRATES = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = PUBLISH_CRATES
SPEC.loader.exec_module(PUBLISH_CRATES)


class PublishCratesTest(unittest.TestCase):
    """验证版本递增和发布 manifest 的最小确定性合同。"""

    def test_bump_version_resets_lower_segments(self) -> None:
        """patch、minor 与 major 必须按 SemVer 清零低位。"""
        self.assertEqual(PUBLISH_CRATES.bump_version("0.1.0", "patch"), "0.1.1")
        self.assertEqual(PUBLISH_CRATES.bump_version("0.1.9", "minor"), "0.2.0")
        self.assertEqual(PUBLISH_CRATES.bump_version("1.9.9", "major"), "2.0.0")
        self.assertGreater(
            PUBLISH_CRATES.version_parts("0.1.1"),
            PUBLISH_CRATES.version_parts("0.1.0"),
        )

    def test_rewrite_manifest_updates_package_and_exact_internal_versions(self) -> None:
        """单次改写必须同时更新 package 与登记过的内部精确依赖。"""
        package = PUBLISH_CRATES.ReleasePackage(
            "helix-driver-native",
            Path("crates/helix-driver-native/Cargo.toml"),
            ("helix-core", "helix-driver-host", "helix-im"),
        )
        source = """[package]
name = "helix-driver-native"
version = "0.1.0"

[dependencies]
helix-core = { package = "cses-helix-core", path = "../helix-core", version = "=0.1.0" }
helix-driver-host = { path = "../helix-driver-host", version = "=0.1.0" }

[dev-dependencies]
helix-im = { path = "../helix-im", version = "=0.1.0" }
"""
        updated = PUBLISH_CRATES.rewrite_manifest(
            source,
            package,
            "0.1.0",
            "0.1.1",
        )
        self.assertIn('version = "0.1.1"', updated)
        self.assertEqual(updated.count('version = "=0.1.1"'), 3)
        self.assertNotIn('version = "=0.1.0"', updated)

    def test_rewrite_manifest_rejects_missing_registered_dependency(self) -> None:
        """登记依赖缺失时必须失败，禁止生成部分同步的版本图。"""
        package = PUBLISH_CRATES.ReleasePackage(
            "helix-im",
            Path("crates/helix-im/Cargo.toml"),
            ("helix-core",),
        )
        source = """[package]
name = "helix-im"
version = "0.1.0"
"""
        with self.assertRaises(PUBLISH_CRATES.ReleaseError):
            PUBLISH_CRATES.rewrite_manifest(source, package, "0.1.0", "0.1.1")

    def test_actual_manifests_form_a_rewritable_release_graph(self) -> None:
        """当前四个真实 manifest 必须能整体改写并解析成同一目标版本。"""
        self.assertEqual(
            [package.cargo_name for package in PUBLISH_CRATES.PACKAGES],
            [
                "cses-helix-core",
                "helix-im",
                "helix-driver-host",
                "helix-driver-native",
            ],
        )
        for package in PUBLISH_CRATES.PACKAGES:
            manifest_path = PUBLISH_CRATES.REPO_ROOT / package.manifest
            source = manifest_path.read_text(encoding="utf-8")
            updated = PUBLISH_CRATES.rewrite_manifest(
                source,
                package,
                "0.1.0",
                "0.1.1",
            )
            document = tomllib.loads(updated)
            self.assertEqual(document["package"]["version"], "0.1.1")
            dependency_tables = (
                document.get("dependencies", {}),
                document.get("dev-dependencies", {}),
            )
            for dependency_name in package.internal_dependencies:
                declarations = [
                    table[dependency_name]
                    for table in dependency_tables
                    if dependency_name in table
                ]
                self.assertEqual(len(declarations), 1)
                self.assertEqual(declarations[0]["version"], "=0.1.1")


if __name__ == "__main__":
    unittest.main()
