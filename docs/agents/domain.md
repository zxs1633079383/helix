# Domain Docs

How the engineering skills should consume this repo's domain documentation when
exploring the codebase. **This repo is single-context** (one workspace, one
domain — the runtime-agnostic deterministic business kernel), but it does **not**
use the stock `CONTEXT.md` + `docs/adr/` layout. The mappings below reflect
helix's actual conventions.

## Before exploring, read these

- **`docs/GOAL.md`** and **`docs/README.md`** — the project's purpose and high-level domain language (helix has no root `CONTEXT.md`; treat these as the context doc).
- **`docs/harness/README.md`** — index of engineering harness cards; the closest thing to a domain/operating glossary.
- **`docs/harness/decisions.md`** — the architectural decision log (ADRs live here, referenced as `ADR-00N`, *not* under `docs/adr/`).
- **`docs/harness/HX-C0NN-*.md`** — individual harness cards capturing hard-won constraints (zero-runtime-dep core, hot-path zero-alloc, FFI boundary discipline, etc.). Read the card(s) touching the area you're about to work in.
- **`specs/` 与 crate-local `docs/`** — 当前 Feature 合同与模块说明。先按
  [`docs/README.md`](../README.md) 路由；旧设计已折叠或删除时，不沿历史路径猜测。

If any of these don't exist for the area you're touching, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates docs lazily when terms or decisions actually get resolved.

## File structure (helix actual)

```
/
├── AGENTS.md                          ← root operating instructions
├── docs/
│   ├── GOAL.md, README.md, ROADMAP.md ← purpose + domain language
│   ├── harness/
│   │   ├── README.md                  ← harness card index
│   │   ├── decisions.md               ← ADR log (ADR-00N)
│   │   ├── OPERATING.md
│   │   └── HX-C0NN-*.md               ← per-constraint harness cards
│   ├── review/, api-baseline/
├── specs/                             ← Spec Kit features and current contracts
└── crates/                            ← Rust workspace (helix-core, helix-im, drivers, …)
```

## Use the project's vocabulary

When your output names a domain concept (an issue title, a refactor proposal, a
hypothesis, a test name), use the term as it appears in `docs/GOAL.md` /
`docs/harness/` / the relevant current Spec or crate-local doc. Don't drift to synonyms.

If the concept you need isn't documented yet, that's a signal — either you're
inventing language the project doesn't use (reconsider) or there's a real gap
(note it for `/domain-modeling`).

## Flag ADR / harness-card conflicts

If your output contradicts an existing ADR in `docs/harness/decisions.md` or a
harness card under `docs/harness/HX-C0NN-*.md`, surface it explicitly rather than
silently overriding:

> _Contradicts ADR-006 (WASM driver paused) — but worth reopening because…_
> _Contradicts HX-C005 (hot-path zero-alloc) — this change allocates on the push path…_
