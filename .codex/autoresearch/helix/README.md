# Helix AutoReSearch Graduation Bundle

This bundle distills Helix from a founder/core-maintainer perspective into a portable project-local knowledge system.

Source scope: live repository docs, crate entry points, scripts, harness index, ADRs, and code graph signals. This is not a trained AutoReSearch bundle, so the score is unscored.

## Contents

- `knowledge/helix_knowledge.md`: full knowledge base.
- `scripts/evaluate.py`: structural self-check.
- `scripts/distill_project.py`: read-only inventory helper for refreshing facts.
- `skills/helix-knowledge/SKILL.md`: Tier 1 Skill.
- `agents/helix-researcher.md`: Tier 2 ReSearch Agent.
- `agents/helix-dev.md`: Tier 3 Dev Agent.
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

The project-local bundle remains here. Installed copies go to the receiver's Codex home through `CODEX_HOME` or the default home setting.

## Keep Training

Use `agents/helix-researcher.md` when ROADMAP, harness cards, ADRs, or verification commands drift. The researcher should update `knowledge/helix_knowledge.md`, then run `python3 scripts/evaluate.py`.

All bundle-internal references are relative so this directory can be copied as a unit.
