# Helix Spec Kit Loop

`helix-loop` extends, but does not replace, the standard Spec Kit flow:

```text
specify -> plan -> tasks -> implement -> analyze/converge
```

New features opt in through the `Loop Profile` marker with value `helix-loop/v1`,
emitted by the project `spec-template.md`. Existing feature bundles remain unmanaged
until they are explicitly migrated.

## Four Enforcement Mechanisms

1. `review-ledger.json` is the persistent workflow state. It records phase,
   gate verdicts, input fingerprints, failure attempts, and the next action.
2. The invalidation graph fingerprints discovery/spec, then plan, then tasks,
   then evidence. Only downstream gates become invalid when an input changes.
3. R1-R4 validators enforce source facts and external completion, technical
   detail and state, task traceability, and hashed verification evidence.
4. Negative tests exercise missing source facts, task-contract omissions,
   digest drift, lifecycle closure, and legacy-feature non-interference.

## Verdicts

- `PASS`: a validator accepted the current gate.
- `READY_FOR_PLAN` / `READY_FOR_TASKS`: the next standard Spec Kit command may run.
- `IMPLEMENTATION_ADMITTED`: R1-R3 are current and implementation may begin.
- `BLOCKED`: repair only the listed owner artifact.
- `NEEDS_HUMAN`: the same failure exceeded its automatic repair budget.
- `TASKS_APPENDED`: convergence changed tasks; return directly to tasking.
- `CLOSED`: R4 passed and convergence changed no task.

Run the installed command as documented in `commands/advance.md`.

## Removal

Preview the inverse change first:

```bash
python3 .specify/extensions/helix-loop/scripts/uninstall_profile.py
```

Then apply it deliberately:

```bash
python3 .specify/extensions/helix-loop/scripts/uninstall_profile.py --apply
```

The uninstaller removes this extension and its hooks, restores only the
profile-managed template and constitution blocks, and preserves `specs/**`.
It refuses to act when one of those managed blocks has been edited.
