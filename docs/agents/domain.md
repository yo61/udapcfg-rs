# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root.
- **`decisions/`**: read decisions that touch the area you're about to work in.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms or decisions actually get resolved.

## File structure

Single-context repo:

```
/
├── CONTEXT.md
├── decisions/
│   └── 2026-09-13-native-runners-over-cross-compilation.md
└── crates/
```

## Decision location and format

This section overrides `domain-modeling/ADR-FORMAT.md`. Wherever a skill says ADR, it means a file in `decisions/`.

- Path: `decisions/YYYY-MM-DD-{topic}.md`, not `docs/adr/NNNN-slug.md`. Never create `docs/adr/`.
- Before writing one, grep `decisions/` for prior decisions in the same area. Follow them unless new information invalidates their reasoning; if replacing one, name it under Supersedes.
- Headings, in order:

```markdown
## Decision: {what was decided}

## Context: {why this came up}

## Alternatives considered: {what else was on the table}

## Reasoning: {why this option won}

## Trade-offs accepted: {what was given up}

## Supersedes: {link to prior decision, if replacing}
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag decision conflicts

If your output contradicts an existing decision, surface it explicitly rather than silently overriding:

> _Contradicts `decisions/2026-09-13-native-runners-over-cross-compilation.md`, but worth reopening because…_

**Fidelity to go-udap v2.4.8 (`43864a5`) outranks the glossary.** Any behavioural divergence not listed in the spec's [Accepted behavioural deltas](../specs/2026-09-08-rust-port-spec.md#accepted-behavioural-deltas) table is a bug. Flag it the same way as a decision conflict, citing the Go source with `git show 43864a5:path/to/file.go`.
