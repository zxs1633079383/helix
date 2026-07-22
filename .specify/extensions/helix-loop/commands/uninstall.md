---
name: "speckit-helix-loop-uninstall"
description: "Safely remove the Helix Spec Kit Loop profile, hooks, and template additions."
compatibility: "Requires the Helix Spec Kit Loop extension to still be installed"
---

## Procedure

1. Preview the exact files that would be restored:

   ```bash
   python3 .specify/extensions/helix-loop/scripts/uninstall_profile.py
   ```

2. Apply the removal only after reviewing that preview:

   ```bash
   python3 .specify/extensions/helix-loop/scripts/uninstall_profile.py --apply
   ```

The command removes the installed extension and hooks, restores only the four
profile-managed Spec Kit files, and removes the project-local extension source.
It never deletes `specs/**`, including feature specifications, plans, tasks,
review ledgers, or verification evidence.

If a managed section was edited after installation, the command stops with
`NEEDS_HUMAN` and writes nothing. Resolve that local change deliberately rather
than forcing a broad directory deletion.
