# Helix AutoReSearch Graduation Bundle

This bundle distills Helix from a founder/core-maintainer perspective into a portable project-local knowledge system.

Source scope: live repository docs, crate entry points, scripts, harness index, ADRs, and code graph signals. This is not a trained AutoReSearch bundle, so the score is unscored.

## Contents

- `knowledge/helix_knowledge.md`: full knowledge base.
- `scripts/evaluate.py`: structural self-check.
- `scripts/distill_project.py`: read-only inventory helper for refreshing facts.
- `skills/helix-knowledge/SKILL.md`: Tier 1 Skill.
- `skills/otel-prometheus-metrics/SKILL.md`: 先评审、低侵入接入并验证 OTel/Prometheus 指标采集的专用 Skill。
- `agents/helix-researcher.md`: Tier 2 ReSearch Agent.
- `agents/helix-dev.md`: Tier 3 Dev Agent.
- `agents/helix-otel-prometheus-metrics-agent.md`: 指标契约、producer 接线与 Prometheus 证据审查 Agent。
- `agents/helix-otel-prometheus-metrics-agent.toml`: Codex 全局 Agent 发现所需的 TOML 适配定义。
- `skills/otel-prometheus-metrics/references/`: 按需加载的当前指标契约和运行证据参考。
- `skills/otel-prometheus-metrics/scripts/verify_prometheus_metric.py`: 只读 metadata/query 验证器。
- `install.sh`: optional copy installer.

## Evaluate

Run from this bundle root:

```bash
python3 scripts/evaluate.py
```

Optional inventory refresh:

```bash
python3 scripts/distill_project.py --project ../../..
```

## Install

Installation is explicit and copy-based:

```bash
bash install.sh
```

The project-local bundle remains here. Installed copies go to the receiver's Codex home through `CODEX_HOME` or the default home setting, and the Skill is also copied to the official user skill directory `~/.agents/skills`. The Skill is copied under `skills/otel-prometheus-metrics`, and the global Agent uses the TOML adapter under `agents/`; start a new task or reload Codex after installation so slash/Agent discovery refreshes.

## Keep Training

Use `agents/helix-researcher.md` when ROADMAP, harness cards, ADRs, or verification commands drift. The researcher should update `knowledge/helix_knowledge.md`, then run `python3 scripts/evaluate.py`.

All bundle-internal references are relative so this directory can be copied as a unit.
