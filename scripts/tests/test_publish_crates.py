"""一键发布脚本的纯版本与 manifest 改写回归。"""

from __future__ import annotations

import importlib.util
import sys
import tomllib
import unittest
from pathlib import Path
from unittest import mock


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

    def test_select_version_skips_full_and_partial_occupied_versions(self) -> None:
        """任何一个包占用都跳过，直到四包都可用。"""
        def exists(name: str, version: str) -> bool:
            return version == "0.1.40" or (
                version == "0.1.41" and name == "helix-driver-native"
            )

        with mock.patch.object(PUBLISH_CRATES, "registry_version_exists", side_effect=exists) as query:
            self.assertEqual(PUBLISH_CRATES.select_target_version("0.1.40"), "0.1.42")
        self.assertEqual(query.call_count, 12)

    def test_select_version_keeps_free_candidate_and_exact_intent(self) -> None:
        """空闲版本保持不变；显式目标不被静默替换。"""
        with mock.patch.object(PUBLISH_CRATES, "registry_version_exists", return_value=False):
            self.assertEqual(PUBLISH_CRATES.select_target_version("0.2.0"), "0.2.0")
        with mock.patch.object(PUBLISH_CRATES, "registry_version_exists", return_value=True):
            with self.assertRaisesRegex(PUBLISH_CRATES.ReleaseError, "显式 --version"):
                PUBLISH_CRATES.select_target_version("0.1.40", exact=True)

    def test_unknown_registry_failure_does_not_bump(self) -> None:
        """网络或认证失败不能当作占用或空闲。"""
        with mock.patch.object(PUBLISH_CRATES, "registry_version_exists", side_effect=PUBLISH_CRATES.ReleaseError("network")) as query:
            with self.assertRaisesRegex(PUBLISH_CRATES.ReleaseError, "network"):
                PUBLISH_CRATES.select_target_version("0.1.40")
            self.assertEqual(query.call_count, 1)

    def test_plan_renders_selected_version_without_writing(self) -> None:
        """编排必须把跳号后的版本交给完整 manifest 图，plan 不写文件。"""
        from argparse import Namespace
        args = Namespace(version=None, bump="patch", publish=False, allow_dirty_plan=False)
        with mock.patch.object(PUBLISH_CRATES, "parse_args", return_value=args), \
             mock.patch.object(PUBLISH_CRATES, "tracked_worktree_is_clean", return_value=True), \
             mock.patch.object(PUBLISH_CRATES, "ensure_no_untracked_package_files"), \
             mock.patch.object(PUBLISH_CRATES, "ensure_current_versions_resolve"), \
             mock.patch.object(PUBLISH_CRATES, "select_target_version", return_value="0.1.99"), \
             mock.patch.object(PUBLISH_CRATES, "render_candidate_manifests", wraps=PUBLISH_CRATES.render_candidate_manifests) as render, \
             mock.patch.object(PUBLISH_CRATES, "write_candidate_manifests") as write:
            self.assertEqual(PUBLISH_CRATES.main(), 0)
            self.assertEqual(render.call_args.args[1], "0.1.99")
            write.assert_not_called()

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
        current_version = PUBLISH_CRATES.current_release_version()
        next_version = PUBLISH_CRATES.bump_version(current_version, "patch")
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
                current_version,
                next_version,
            )
            document = tomllib.loads(updated)
            self.assertEqual(document["package"]["version"], next_version)
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
                self.assertEqual(declarations[0]["version"], f"={next_version}")

    def test_candidate_transaction_rewrites_all_workspace_exact_consumers(self) -> None:
        """候选事务必须同步非发布消费者，避免 Cargo workspace 解析版本冲突。"""
        current_version = PUBLISH_CRATES.current_release_version()
        next_version = PUBLISH_CRATES.bump_version(current_version, "patch")
        rendered = PUBLISH_CRATES.render_candidate_manifests(current_version, next_version)
        consumers = (
            Path("crates/helix-driver-web/Cargo.toml"),
            Path("crates/helix-driver-ffi/Cargo.toml"),
            Path("crates/helix-host-cli/Cargo.toml"),
            Path("tools/helix-perf/Cargo.toml"),
        )
        for manifest in consumers:
            current_document = tomllib.loads(
                (PUBLISH_CRATES.REPO_ROOT / manifest).read_text(encoding="utf-8")
            )
            document = tomllib.loads(rendered[manifest])
            self.assertEqual(
                document["package"]["version"],
                current_document["package"]["version"],
            )
            self.assertEqual(
                document["dependencies"]["helix-core"]["version"],
                f"={next_version}",
            )

    def test_candidate_transaction_keeps_publish_set_separate(self) -> None:
        """非发布消费者参与版本事务，但绝不能进入 crates.io 上传集合。"""
        self.assertEqual(len(PUBLISH_CRATES.PACKAGES), 4)
        self.assertEqual(
            set(PUBLISH_CRATES.transaction_manifest_paths()),
            {
                Path("crates/helix-core/Cargo.toml"),
                Path("crates/helix-im/Cargo.toml"),
                Path("crates/helix-driver-host/Cargo.toml"),
                Path("crates/helix-driver-native/Cargo.toml"),
                Path("crates/helix-driver-web/Cargo.toml"),
                Path("crates/helix-driver-ffi/Cargo.toml"),
                Path("crates/helix-host-cli/Cargo.toml"),
                Path("tools/helix-perf/Cargo.toml"),
            },
        )

    def test_candidate_transaction_rejects_unregistered_exact_dependency(self) -> None:
        """workspace 新增精确 path 引用但未登记时，候选计划必须 fail-closed。"""
        discovered = PUBLISH_CRATES.discover_exact_internal_dependencies()
        discovered[(Path("Cargo.toml"), "helix-core-shadow")] = "=0.1.0"
        with mock.patch.object(
            PUBLISH_CRATES,
            "discover_exact_internal_dependencies",
            return_value=discovered,
        ):
            with self.assertRaisesRegex(
                PUBLISH_CRATES.ReleaseError,
                "unregistered",
            ):
                PUBLISH_CRATES.render_candidate_manifests("0.1.0", "0.1.1")


if __name__ == "__main__":
    unittest.main()
