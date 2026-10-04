# 0001 — Build a thin end-to-end slice before finishing Phase 0

Date: 2026-10-04 · Status: accepted

## Context

The plan (`build.md` §7) builds the parser completely (Phase 0), then types,
then effects, then the interpreter. The goal "run a hello world test" needs a
piece of every stage, so the effect system's core claims could be exercised
end to end early.

## Decision

Build a minimal vertical slice now:

- **`nova_syntax`**: a hand-written lexer and recursive-descent parser producing
  an owned AST. Covers `fn`, `effect`, `test`, `let`/`var`, assignment, `if`,
  blocks, calls with named arguments, string interpolation, lists, `return`,
  `handle … with { … }` (inline handlers) and `abort`. Everything else
  specified fails with E0009 naming its phase.
- **`nova_types`**: effect tree, operation resolution (effects §5.2), effect
  inference with fixpoint and provenance (§7), handler checking (§6), root checks
  (§7.6). **No type checking yet.**
- **`nova_interp`**: tree-walking interpreter with deep, tail-resumptive
  handlers, delegation, and `abort` (§8); a `console` root handler.
- **`nova_cli`**: `nova run`, `nova test`, `nova check`.
- No external dependencies.

## Consequences

- P0-05/P0-07 still owe a **lossless rowan CST**; the current parser drops
  comments and is not suitable for `nova fmt` or the LSP. The recursive-descent
  structure is meant to be ported to the event-based parser, not thrown away.
- Newline rules and interpolation syntax are **provisional** until P0-02
  (`docs/spec/syntax.md`) is approved; implemented rules are documented at the
  top of `crates/nova_syntax/src/parser.rs`.
- Without types, type errors (`1 + "a"`) are runtime panics. Phase 1 replaces this.
- `ariadne` was not adopted yet; `nova_diag` has a small plain renderer.
