# Nova — agent guide

Nova is a statically typed language with an effect system and first-class AI.
Read `docs/build.md` (plan), `docs/spec/` (normative specs), and the current
phase file in `docs/phases/` before changing anything.

## Commands

- Build: `cargo build`
- Test everything: `cargo test` (unit tests, `tests/ui`, `tests/run`, CLI tests)
- Lint: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
- Try it: `nova examples/hello_world.nova` and `nova test examples/hello_world.nova` (install with `cargo install --path crates/nova_cli`)

## Layout

| Crate | Role | May depend on |
|---|---|---|
| `nova_diag` | spans, source map, diagnostics, rendering | — |
| `nova_syntax` | lexer, AST, parser | `nova_diag` |
| `nova_types` | effect tree, name resolution, effect checking | `nova_diag`, `nova_syntax` |
| `nova_interp` | tree-walking interpreter, handlers, root handlers | the three above |
| `nova_cli` | `nova` binary (`run`, `test`, `check`) | all |

- `std/effects.nova` — built-in effect declarations (embedded by `nova_types`).
- `tests/ui/**/*.nova` — checker tests; annotate expected diagnostics with
  `//~ ERROR E0402` (same line) or `//~^ WARN W0401` (line above). Files with no
  annotations must produce no diagnostics.
- `tests/run/*.nova` + `.stdout` — programs run with `nova run`; stdout must match.
- `examples/` — runnable examples; keep them passing.

## Rules

- The spec is the contract. Cite the `docs/spec/` section you implement. If the
  spec is ambiguous or silent, stop and add the question to
  `docs/syntax-questions.md` instead of inventing syntax or semantics.
- Tests first: add failing UI/run/unit tests, then implement.
- Every diagnostic has a code registered in `docs/spec/errors.md`.
- Keep tasks small (≤ ~500 changed lines). No new dependencies without a
  decision record in `docs/decisions/`.
- Unsupported-but-specified syntax must fail with `E0009` naming the phase that
  delivers it — never silently misparse.
