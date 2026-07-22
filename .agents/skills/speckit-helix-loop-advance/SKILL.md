---
name: speckit-helix-loop-advance
description: Advance the active Spec Kit feature through a persistent, evidence-backed
  loop gate.
compatibility: Requires spec-kit project structure with .specify/ directory
metadata:
  author: github-spec-kit
  source: helix-loop:commands/advance.md
---

## Procedure

1. Run:

   ```bash
   python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance
   ```

2. Read the emitted JSON verdict.
   - `PASS`, `REUSED`, `READY_FOR_PLAN`, `READY_FOR_TASKS`, and
     `IMPLEMENTATION_ADMITTED` may proceed to the next Spec Kit phase.
   - `BLOCKED` means repair only the listed artifact and rerun this command.
   - `NEEDS_HUMAN` means stop and ask for the recorded decision.
   - `TASKS_APPENDED` means return to `/speckit-tasks`; do not rerun prior reviews.
   - `CLOSED` is the only completion verdict.
3. Never change source code, task state, or evidence merely to make this command pass.
   Record source facts, plan detail, task contracts, or real verification outputs in
   their owning artifacts instead.

## Evidence Recording

After a real command produces an output file under the active feature's `evidence/`
directory, register it before running the after-implementation gate:

```bash
python3 .specify/extensions/helix-loop/scripts/loop_engine.py record-evidence \
  --id V-001 \
  --command '<actual command>' \
  --started-at '<ISO-8601 timestamp>' \
  --ended-at '<ISO-8601 timestamp>' \
  --result PASS \
  --file 'specs/<feature>/evidence/<output-file>'
```

The command hashes the output and rejects missing or out-of-scope evidence files.