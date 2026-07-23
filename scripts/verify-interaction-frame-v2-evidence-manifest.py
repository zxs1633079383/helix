#!/usr/bin/env python3
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "specs/018-interaction-frame-v2-cutover/contracts/evidence-manifest-v2.md"
EXPECTED = [
    "G-S1", "G-S2", "G-S3", "G-S4",
    "G-01", "G-02", "G-04", "G-04a", "G-05", "G-06", "G-07",
    "G-08", "G-09", "G-10", "G-10a", "G-11", "G-12", "G-13",
    "G-14", "G-14a", "G-15a", "G-15", "G-16", "G-16b", "G-16c",
]


def main() -> int:
    text = MANIFEST.read_text(encoding="utf-8")
    rows = re.findall(r"^\| (G-(?:S[1-4]|\d{2}[a-c]?)) \|(.+)\|$", text, re.MULTILINE)
    gate_ids = [gate_id for gate_id, _ in rows]
    errors = []
    if gate_ids != EXPECTED:
        errors.append(f"gate order mismatch: expected={EXPECTED} actual={gate_ids}")
    for gate_id, body in rows:
        if "`" not in body or body.count("|") < 2:
            errors.append(f"{gate_id}: missing database key or before/after delta")
    if "G-03" in gate_ids or "G-03a" in gate_ids:
        errors.append("G03 dimensions must not enter the 25-Gate profile")
    if len(set(gate_ids)) != len(gate_ids):
        errors.append("duplicate Gate contract")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"evidence-manifest-v2: PASS ({len(gate_ids)} Gate contracts)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
