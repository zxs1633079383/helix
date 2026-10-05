#!/usr/bin/env python3
"""按依赖拓扑递增并发布 Helix 公共 crates。"""

from __future__ import annotations

import argparse
import re
import shlex
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
EXPECTED_LICENSE = "MIT OR Apache-2.0"
REGISTRY_ATTEMPTS = 12
REGISTRY_DELAY_SECONDS = 5


class ReleaseError(RuntimeError):
    """表示发布前置、校验或远端状态不满足安全继续条件。"""


@dataclass(frozen=True)
class ReleasePackage:
    """描述一个发布单元及其需要同步的仓内依赖 key。"""

    cargo_name: str
    manifest: Path
    internal_dependencies: tuple[str, ...] = ()


@dataclass(frozen=True)
class VersionConsumer:
    """描述只参与版本事务、但不会上传 crates.io 的 workspace manifest。"""

    manifest: Path
    internal_dependencies: tuple[str, ...]


PACKAGES = (
    ReleasePackage("cses-helix-core", Path("crates/helix-core/Cargo.toml")),
    ReleasePackage(
        "helix-im",
        Path("crates/helix-im/Cargo.toml"),
        ("helix-core",),
    ),
    ReleasePackage(
        "helix-driver-host",
        Path("crates/helix-driver-host/Cargo.toml"),
        ("helix-core", "helix-im"),
    ),
    ReleasePackage(
        "helix-driver-native",
        Path("crates/helix-driver-native/Cargo.toml"),
        ("helix-core", "helix-driver-host", "helix-im"),
    ),
)

CONSUMERS = (
    VersionConsumer(
        Path("crates/helix-driver-web/Cargo.toml"),
        ("helix-core",),
    ),
    VersionConsumer(
        Path("crates/helix-driver-ffi/Cargo.toml"),
        ("helix-core",),
    ),
    VersionConsumer(
        Path("crates/helix-host-cli/Cargo.toml"),
        ("helix-core",),
    ),
    VersionConsumer(
        Path("tools/helix-perf/Cargo.toml"),
        ("helix-core",),
    ),
)

ManifestTarget = ReleasePackage | VersionConsumer
DEPENDENCY_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def run_command(
    arguments: list[str],
    *,
    capture: bool = False,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    """在仓根执行命令，默认继承输出且不回显任何环境凭据。"""
    print(f"\n$ {shlex.join(arguments)}", flush=True)
    completed = subprocess.run(
        arguments,
        cwd=REPO_ROOT,
        text=True,
        capture_output=capture,
        check=False,
    )
    if check and completed.returncode != 0:
        if capture:
            if completed.stdout.strip():
                print(completed.stdout.rstrip())
            if completed.stderr.strip():
                print(completed.stderr.rstrip(), file=sys.stderr)
        raise ReleaseError(
            f"命令失败（exit={completed.returncode}）：{shlex.join(arguments)}"
        )
    return completed


def bump_version(version: str, bump: str) -> str:
    """对稳定三段式版本执行 patch、minor 或 major 递增。"""
    major, minor, patch = version_parts(version)
    if bump == "patch":
        patch += 1
    elif bump == "minor":
        minor += 1
        patch = 0
    elif bump == "major":
        major += 1
        minor = 0
        patch = 0
    else:
        raise ReleaseError(f"不支持的递增类型：{bump}")
    return f"{major}.{minor}.{patch}"


def version_parts(version: str) -> tuple[int, int, int]:
    """把稳定三段式版本解析为可比较的整数元组。"""
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    if not match:
        raise ReleaseError(f"仅支持稳定三段式版本，当前为 {version!r}")
    return tuple(int(part) for part in match.groups())


def package_document(package: ReleasePackage) -> dict:
    """读取单个发布 manifest，并把 TOML 语法错误提升为发布失败。"""
    manifest_path = REPO_ROOT / package.manifest
    try:
        return tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ReleaseError(f"无法读取 {package.manifest}：{error}") from error


def current_release_version() -> str:
    """确认四个发布包版本与双许可证完全一致并返回同步版本。"""
    versions: dict[str, str] = {}
    for package in PACKAGES:
        document = package_document(package)
        package_table = document.get("package", {})
        versions[package.cargo_name] = str(package_table.get("version", ""))
        if package_table.get("name") != package.cargo_name:
            raise ReleaseError(
                f"{package.manifest} package.name 不是 {package.cargo_name}"
            )
        if package_table.get("license") != EXPECTED_LICENSE:
            raise ReleaseError(
                f"{package.manifest} license 必须为 {EXPECTED_LICENSE}"
            )
    distinct = set(versions.values())
    if len(distinct) != 1:
        details = ", ".join(f"{name}={version}" for name, version in versions.items())
        raise ReleaseError(f"四个发布包版本不一致：{details}")
    version = distinct.pop()
    bump_version(version, "patch")
    return version


def rewrite_manifest(
    content: str,
    target: ManifestTarget,
    old_version: str,
    new_version: str,
    *,
    update_package_version: bool = True,
) -> str:
    """按目标声明改写 package.version 和仓内精确依赖版本。"""
    lines = content.splitlines(keepends=True)
    section = ""
    package_version_updated = not update_package_version
    dependency_updates = {name: 0 for name in target.internal_dependencies}
    package_version_pattern = re.compile(
        rf'^(version\s*=\s*"){re.escape(old_version)}("\s*)$'
    )

    for index, line in enumerate(lines):
        stripped = line.rstrip("\r\n")
        newline = line[len(stripped) :]
        section_match = re.fullmatch(r"\s*\[([^]]+)]\s*", stripped)
        if section_match:
            section = section_match.group(1)
            continue
        if (
            update_package_version
            and section == "package"
            and not package_version_updated
        ):
            replaced, count = package_version_pattern.subn(
                rf'\g<1>{new_version}\g<2>', stripped
            )
            if count:
                lines[index] = replaced + newline
                package_version_updated = True
                continue
        for dependency_name in dependency_updates:
            if not re.match(rf"^\s*{re.escape(dependency_name)}\s*=", stripped):
                continue
            replaced, count = re.subn(
                rf'(\bversion\s*=\s*"=){re.escape(old_version)}(")',
                rf'\g<1>{new_version}\g<2>',
                stripped,
            )
            if count != 1:
                raise ReleaseError(
                    f"{target.manifest} 的 {dependency_name} 没有唯一的 "
                    f'version = "={old_version}"'
                )
            lines[index] = replaced + newline
            dependency_updates[dependency_name] += 1
            break

    if not package_version_updated:
        raise ReleaseError(
            f"{target.manifest} 未找到 package.version = {old_version}"
        )
    invalid_counts = {
        name: count for name, count in dependency_updates.items() if count != 1
    }
    if invalid_counts:
        raise ReleaseError(
            f"{target.manifest} 仓内依赖更新次数异常：{invalid_counts}"
        )
    return "".join(lines)


def transaction_manifest_paths() -> tuple[Path, ...]:
    """返回需要原子改写和回滚的发布包与非发布消费者 manifest。"""
    return tuple(package.manifest for package in PACKAGES) + tuple(
        consumer.manifest for consumer in CONSUMERS
    )


def workspace_manifest_paths() -> tuple[Path, ...]:
    """返回根 manifest 与全部 workspace 成员 manifest，拒绝静默漏扫。"""
    root_document = tomllib.loads(
        (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
    )
    members = root_document.get("workspace", {}).get("members", [])
    manifests = (Path("Cargo.toml"),) + tuple(
        Path(member) / "Cargo.toml" for member in members
    )
    missing = [str(path) for path in manifests if not (REPO_ROOT / path).is_file()]
    if missing:
        raise ReleaseError("workspace member manifest 不存在：" + ", ".join(missing))
    return manifests


def dependency_tables(document: dict) -> tuple[dict, ...]:
    """收集普通、workspace 与 target 条件下的 Cargo dependency 表。"""
    tables = [
        table
        for section in DEPENDENCY_SECTIONS
        if isinstance((table := document.get(section)), dict)
    ]
    targets = document.get("target", {})
    if isinstance(targets, dict):
        for target_document in targets.values():
            if not isinstance(target_document, dict):
                continue
            tables.extend(
                table
                for section in DEPENDENCY_SECTIONS
                if isinstance((table := target_document.get(section)), dict)
            )
    workspace = document.get("workspace", {})
    if isinstance(workspace, dict) and isinstance(
        workspace.get("dependencies"),
        dict,
    ):
        tables.append(workspace["dependencies"])
    return tuple(tables)


def discover_exact_internal_dependencies() -> dict[tuple[Path, str], str]:
    """按依赖 path 识别指向公共 crate 的 workspace 精确版本声明。"""
    published_manifests = {
        (REPO_ROOT / package.manifest).resolve()
        for package in PACKAGES
    }
    discovered: dict[tuple[Path, str], str] = {}
    for manifest in workspace_manifest_paths():
        manifest_path = REPO_ROOT / manifest
        document = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        for table in dependency_tables(document):
            for dependency_name, declaration in table.items():
                if not isinstance(declaration, dict) or "path" not in declaration:
                    continue
                dependency_manifest = (
                    manifest_path.parent / str(declaration["path"]) / "Cargo.toml"
                ).resolve()
                if dependency_manifest not in published_manifests:
                    continue
                version = declaration.get("version")
                if isinstance(version, str) and version.startswith("="):
                    key = (manifest, dependency_name)
                    if key in discovered:
                        raise ReleaseError(
                            f"{manifest} 的 {dependency_name} 存在多个精确 path 声明"
                        )
                    discovered[key] = version
    return discovered


def ensure_version_transaction_complete(old_version: str) -> None:
    """确认所有公共 crate 精确 path 引用均已登记进版本事务。"""
    registered = {
        (target.manifest, dependency_name)
        for target in (*PACKAGES, *CONSUMERS)
        for dependency_name in target.internal_dependencies
    }
    discovered = discover_exact_internal_dependencies()
    discovered_keys = set(discovered)
    missing = sorted(
        registered - discovered_keys,
        key=lambda item: (str(item[0]), item[1]),
    )
    unregistered = sorted(
        discovered_keys - registered,
        key=lambda item: (str(item[0]), item[1]),
    )
    mismatched = sorted(
        (manifest, dependency, version)
        for (manifest, dependency), version in discovered.items()
        if version != f"={old_version}"
    )
    if missing or unregistered or mismatched:
        raise ReleaseError(
            "workspace 精确 path 依赖未形成完整版本事务："
            f"missing={missing}, unregistered={unregistered}, mismatched={mismatched}"
        )


def render_candidate_manifests(old_version: str, new_version: str) -> dict[Path, str]:
    """在内存中生成完整候选版本图，供 plan 和真实发布共用。"""
    ensure_version_transaction_complete(old_version)
    rendered: dict[Path, str] = {}
    for package in PACKAGES:
        source = (REPO_ROOT / package.manifest).read_text(encoding="utf-8")
        rendered[package.manifest] = rewrite_manifest(
            source,
            package,
            old_version,
            new_version,
        )
    for consumer in CONSUMERS:
        source = (REPO_ROOT / consumer.manifest).read_text(encoding="utf-8")
        rendered[consumer.manifest] = rewrite_manifest(
            source,
            consumer,
            old_version,
            new_version,
            update_package_version=False,
        )
    for manifest, content in rendered.items():
        try:
            tomllib.loads(content)
        except tomllib.TOMLDecodeError as error:
            raise ReleaseError(f"候选 manifest 无法解析 {manifest}：{error}") from error
    return rendered


def tracked_worktree_is_clean() -> bool:
    """只检查 tracked diff，允许仓库既有且不会入包的未跟踪证据目录。"""
    completed = run_command(
        ["git", "status", "--porcelain", "--untracked-files=no"],
        capture=True,
    )
    return not completed.stdout.strip()


def ensure_no_untracked_package_files() -> None:
    """拒绝 crate 根内未跟踪文件，避免发布白名单意外携带本机内容。"""
    crate_roots = sorted({str(package.manifest.parent) for package in PACKAGES})
    completed = run_command(
        [
            "git",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            *crate_roots,
        ],
        capture=True,
    )
    untracked = [
        line[3:]
        for line in completed.stdout.splitlines()
        if line.startswith("?? ")
    ]
    if untracked:
        raise ReleaseError(
            "待发布 crate 内存在未跟踪文件：" + ", ".join(untracked)
        )


def registry_version_exists(crate_name: str, version: str) -> bool:
    """区分 registry 已存在、明确不存在与网络/认证类未知失败。"""
    completed = run_command(
        [
            "cargo",
            "info",
            "--registry",
            "crates-io",
            f"{crate_name}@{version}",
        ],
        capture=True,
        check=False,
    )
    if completed.returncode == 0:
        return True
    combined = f"{completed.stdout}\n{completed.stderr}".lower()
    if "could not find" in combined:
        return False
    raise ReleaseError(
        f"无法确定 {crate_name}@{version} 的 registry 状态：\n"
        f"{completed.stderr.strip() or completed.stdout.strip()}"
    )


def select_target_version(version: str, *, exact: bool = False) -> str:
    """上传前选择四包均空闲的版本；显式版本严格匹配，未知 registry 错误不跳过。"""
    while True:
        occupied = [
            package.cargo_name
            for package in PACKAGES
            if registry_version_exists(package.cargo_name, version)
        ]
        if not occupied:
            return version
        if exact:
            raise ReleaseError(
                f"目标版本 {version} 已被占用：{', '.join(occupied)}；"
                "显式 --version 不自动改写，请省略该参数以自动选择"
            )
        next_version = bump_version(version, "patch")
        print(
            f"[version] {version} 已被占用（{', '.join(occupied)}），"
            f"自动尝试 {next_version}",
            flush=True,
        )
        version = next_version


def ensure_current_versions_resolve(version: str) -> None:
    """确认本地基线四包均已发布，拒绝从 partial 状态继续自动递增。"""
    missing = [
        package.cargo_name
        for package in PACKAGES
        if not registry_version_exists(package.cargo_name, version)
    ]
    if missing:
        raise ReleaseError(
            f"当前版本 {version} 尚未全部在 registry 可解析：{', '.join(missing)}；"
            "请先处理上一次 partial 发布"
        )


def wait_until_registry_resolves(crate_name: str, version: str) -> None:
    """上传后有界轮询 cargo registry，成功解析前不发布下游 crate。"""
    for attempt in range(1, REGISTRY_ATTEMPTS + 1):
        if registry_version_exists(crate_name, version):
            print(f"[registry] {crate_name}@{version} 已可解析")
            return
        if attempt < REGISTRY_ATTEMPTS:
            print(
                f"[registry] 等待 {crate_name}@{version} "
                f"({attempt}/{REGISTRY_ATTEMPTS})"
            )
            time.sleep(REGISTRY_DELAY_SECONDS)
    raise ReleaseError(
        f"{crate_name}@{version} 上传后仍无法在 registry 解析，停止发布下游"
    )


def snapshot_release_files() -> dict[Path, bytes]:
    """保存事务 manifest 与锁文件，以便首次上传前失败时精确恢复。"""
    paths = [REPO_ROOT / manifest for manifest in transaction_manifest_paths()]
    paths.append(REPO_ROOT / "Cargo.lock")
    return {path: path.read_bytes() for path in paths}


def restore_release_files(snapshots: dict[Path, bytes]) -> None:
    """按字节恢复发布前快照，不影响未登记文件。"""
    for path, content in snapshots.items():
        path.write_bytes(content)


def write_candidate_manifests(rendered: dict[Path, str]) -> None:
    """一次写入已完成内存校验的发布包与消费者候选 manifest。"""
    for manifest, content in rendered.items():
        (REPO_ROOT / manifest).write_text(content, encoding="utf-8")


def assert_release_diff_scope() -> None:
    """确认版本改写后 tracked diff 没有越出 manifest 与 Cargo.lock 白名单。"""
    completed = run_command(
        ["git", "status", "--porcelain", "--untracked-files=no"],
        capture=True,
    )
    allowed = {str(manifest) for manifest in transaction_manifest_paths()} | {
        "Cargo.lock"
    }
    changed = {
        line[3:]
        for line in completed.stdout.splitlines()
        if len(line) >= 4
    }
    unexpected = sorted(changed - allowed)
    if unexpected:
        raise ReleaseError(f"版本改写越出允许范围：{', '.join(unexpected)}")


def validate_release_candidate() -> None:
    """更新锁文件并一次性编译发布包及其 workspace 消费者。"""
    run_command(["cargo", "metadata", "--format-version", "1", "--no-deps"])
    assert_release_diff_scope()
    run_command(
        [
            "cargo",
            "check",
            "-p",
            "cses-helix-core",
            "-p",
            "helix-im",
            "-p",
            "helix-driver-host",
            "-p",
            "helix-driver-native",
            "-p",
            "helix-driver-web",
            "-p",
            "helix-driver-ffi",
            "-p",
            "helix-host-cli",
            "-p",
            "helix-perf",
        ]
    )


def dry_run_package(package: ReleasePackage) -> None:
    """在不可逆上传前验证单包发布内容与 registry 依赖闭包。"""
    run_command(
        ["cargo", "publish", "-p", package.cargo_name, "--dry-run", "--allow-dirty"]
    )


def upload_package(package: ReleasePackage, version: str) -> None:
    """执行单包上传，并辨认命令失败但远端实际已创建版本的边界。"""
    completed = run_command(
        ["cargo", "publish", "-p", package.cargo_name, "--allow-dirty"],
        check=False,
    )
    if completed.returncode != 0:
        if registry_version_exists(package.cargo_name, version):
            print(
                f"[registry] {package.cargo_name}@{version} 命令返回失败，"
                "但远端版本已存在，按已上传继续核验",
                file=sys.stderr,
            )
            return
        raise ReleaseError(f"发布 {package.cargo_name}@{version} 失败（exit={completed.returncode}）")


def print_client_dependencies(version: str) -> None:
    """输出客户端可直接复制的同批精确依赖声明。"""
    print("\n客户端 Cargo.toml：")
    print(
        'helix-core = { package = "cses-helix-core", '
        f'version = "={version}" }}'
    )
    print(f'helix-im = "={version}"')
    print(f'helix-driver-host = "={version}"')
    print(f'helix-driver-native = "={version}"')


def parse_args() -> argparse.Namespace:
    """解析发布模式与版本递增参数。"""
    parser = argparse.ArgumentParser(
        description="递增并按依赖顺序发布四个 Helix crates"
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--publish",
        action="store_true",
        help="执行版本改写、dry-run 和真实发布",
    )
    mode.add_argument(
        "--plan",
        action="store_true",
        help="只检查版本与 registry 占用，不修改文件",
    )
    parser.add_argument(
        "--bump",
        choices=("patch", "minor", "major"),
        default="patch",
        help="递增类型，默认 patch；候选被占用时继续递增 patch",
    )
    parser.add_argument(
        "--version",
        help="显式目标版本；严格匹配，若已占用则失败",
    )
    parser.add_argument(
        "--allow-dirty-plan",
        action="store_true",
        help="仅 plan 模式允许 tracked dirty，真实发布始终拒绝",
    )
    return parser.parse_args()


def main() -> int:
    """编排版本预检、可回滚准备、拓扑发布和最终客户端提示。"""
    args = parse_args()
    if args.allow_dirty_plan and args.publish:
        raise ReleaseError("--allow-dirty-plan 不能用于真实发布")
    old_version = current_release_version()
    new_version = args.version or bump_version(old_version, args.bump)
    if new_version == old_version:
        raise ReleaseError("目标版本必须不同于当前版本")
    if version_parts(new_version) <= version_parts(old_version):
        raise ReleaseError("目标版本必须高于当前版本")

    if not tracked_worktree_is_clean() and not args.allow_dirty_plan:
        raise ReleaseError("存在 tracked dirty；请先提交或恢复后再发布")
    ensure_no_untracked_package_files()
    ensure_version_transaction_complete(old_version)
    ensure_current_versions_resolve(old_version)
    new_version = select_target_version(new_version, exact=args.version is not None)
    print(f"Helix release: {old_version} -> {new_version}")
    rendered = render_candidate_manifests(old_version, new_version)

    if not args.publish:
        print(
            "PLAN PASS：workspace 候选版本图完整且目标版本未占用；"
            "真实发布请运行同命令并增加 --publish"
        )
        return 0

    snapshots = snapshot_release_files()
    published: list[str] = []
    try:
        write_candidate_manifests(rendered)
        validate_release_candidate()
        for package in PACKAGES:
            dry_run_package(package)
            upload_package(package, new_version)
            published.append(package.cargo_name)
            wait_until_registry_resolves(package.cargo_name, new_version)
    except Exception:
        if published:
            print(
                "\nPARTIAL：已有 crate 不可逆上传，保留新版本文件；"
                f"已发布：{', '.join(published)}",
                file=sys.stderr,
            )
        else:
            restore_release_files(snapshots)
            print("\n发布前失败：已恢复 manifest 与 Cargo.lock", file=sys.stderr)
        raise

    print(f"\nRELEASE PASS：四个 crate 的 {new_version} 均已发布并可解析")
    print("未执行 git commit、tag 或 push；请审核版本 diff 后提交。")
    print_client_dependencies(new_version)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ReleaseError as error:
        print(f"RELEASE FAILED: {error}", file=sys.stderr)
        raise SystemExit(1)
