# Nova Spec — Effects

Status: draft v0.1 — 2026-10-04 · Normative
Owner area: `nova_types` (checking), `nova_interp` (semantics), `nova_runtime` (root handlers)
Related: `docs/build.md` §4 (overview), `docs/spec/ai.md` (the `ai` and `approval` effects)

Where this document and `build.md` disagree, this document wins. Sections marked **[MVP]** are required for v0.1. Sections marked **[P7]** or later are specified now so the design holds together, but implementations must reject them with the listed diagnostic until their phase.

---

## 1. Overview

An **effect** is a named, hierarchical set of **operations**. A function's type includes an **effect row**: the set of effects it may perform when called.

- Rows are inferred for private code and declared for public code.
- Performing an operation is dispatched at runtime to the nearest enclosing **handler** for that effect.
- At the program root, handlers are provided by the runtime, but only for the effects `main` declares.

Guarantees:

1. **Soundness.** A call can only perform operations whose effects are in its row.
2. **Completeness at the root.** Every effect a program may perform has a handler. Unhandled effects are compile errors, never runtime failures.
3. **Locality.** Reading a `pub` signature tells you everything the function may do, except panicking, non-termination, and the documented runtime exceptions in §11.

---

## 2. Terminology

| Term | Meaning |
|---|---|
| Effect node | A declared `effect`. Either an **interior** node (has sub-effects) or a **leaf** node (has operations). |
| Effect path | Dotted name of a node from its root, e.g. `fs.read`. |
| Leaf set | The set of leaf nodes under a node. `leaves(fs) = {fs.read, fs.write}`. A leaf's leaf set is itself. |
| Row (φ) | A finite set of leaf paths plus a finite set of effect variables. Written `[fs.read, network.http, E]`. |
| Effect variable | A placeholder for an unknown row (ρ, or a named `effect E` parameter). |
| Operation (op) | A function signature declared inside a leaf effect. |
| Perform | Invoking an operation. |
| Handler | A value or inline block that implements every operation of one effect node's subtree. |
| Root handler | A handler installed by the runtime before `main` (or a test/eval) runs. |

---

## 3. Declaring effects [MVP]

### 3.1 Grammar

```ebnf
EffectDecl    = { DocComment } { Attribute } [ "pub" ] "effect" Ident "{" { EffectMember } "}"
EffectMember  = NestedEffect | OpDecl
NestedEffect  = { DocComment } "effect" Ident "{" { EffectMember } "}"
OpDecl        = { DocComment } "fn" Ident "(" [ Param { "," Param } [ "," ] ] ")" [ "->" Type ]
```

### 3.2 Rules

1. **E0405:** a node contains either operations or nested effects, never both.
2. A node with no members is an error (**E0405**).
3. Nested effects take the visibility of their root. The ops of a `pub effect` are public.
4. Op names are unique within a leaf (**E0416** duplicate op). They may repeat across leaves, which affects call resolution (§5.2).
5. Op signatures may not be generic (**E0415**). Generic ops would force every handler to be generic. APIs that need typed results (e.g. `ai.generate<T>`) use untyped ops plus generic wrapper functions; see `ai.md` §3.
6. Op parameter and return types may mention any type in scope except effect variables.
7. Op signatures carry no `uses` clause. The effects of *implementing* an op belong to the handler (§6.4).
8. The hierarchy is **closed**: other modules cannot add children to an effect. Adding a child in a new library version widens what `uses [parent]` means, and lockfiles report it (§10).
9. The root name `ffi` is reserved (§9.3).

### 3.3 Example

```nova
/// Persistent key-value storage.
pub effect kv {
    effect read {
        fn get(key: String) -> Result<Bytes?, KvError>
        fn scan(prefix: String) -> Result<List<String>, KvError>
    }
    effect write {
        fn put(key: String, value: Bytes) -> Result<(), KvError>
        fn delete(key: String) -> Result<(), KvError>
    }
}
```

`leaves(kv) = {kv.read, kv.write}`.

### 3.4 Guideline

Operations that a sandbox might deny should return `Result<_, E>`, where `E` has a `Denied(reason: String)` variant. All built-in effects follow this (§9).

---

## 4. Rows and `uses` clauses [MVP]

### 4.1 Grammar

```ebnf
UsesClause   = "uses" "[" [ EffectRef { "," EffectRef } [ "," ] ] "]"
EffectRef    = EffectPath | Ident          (* an Ident naming an `effect` generic parameter *)
EffectPath   = Ident { "." Ident }
```

A `uses` clause may appear on:
- `fn` items, including methods, trait methods, and `tool fn`
- function types
- `handler` declarations
- effect arguments in generic argument lists (§4.5)

### 4.2 Normalization

Before any comparison, a row is normalized:

1. Every path is replaced by its leaf set (`fs` → `fs.read, fs.write`).
2. Duplicates are removed.

Diagnostics display rows **compacted**:
- a complete leaf set collapses back to its parent (`fs.read, fs.write` → `fs`), applied bottom-up
- leaves are listed alphabetically, followed by variables

### 4.3 Subsumption

`φ₁ ⊑ φ₂` holds iff `leaves(φ₁) ⊆ leaves(φ₂)` and `vars(φ₁) ⊆ vars(φ₂)`.

The special path `ffi` subsumes everything: `φ ⊑ [ffi]` for all φ (§9.3).

### 4.4 Function types

```nova
fn(String) -> Int                          // row omitted
fn(String) -> Int uses []                  // explicitly pure
fn(Url) -> Bytes uses [network.http]
```

- Function types are **covariant in their row**: `fn(A) -> B uses φ₁` is a subtype of `fn(A) -> B uses φ₂` iff `φ₁ ⊑ φ₂`. This is the only subtyping in Nova besides `mut T → T` and `Never`.
- Meaning of an **omitted** row, by position:

| Position | Omitted row means |
|---|---|
| Parameter type of a `fn` item (top level of the parameter's type) | Fresh implicit effect variable (§4.6) |
| Anywhere else: return types, fields, `let` annotations, nested inside parameter types, type arguments | `uses []` (pure) |

So an effectful closure stored in a field or returned from a function must say so: `handler_fn: fn(Event) uses [console]`.

### 4.5 Effect parameters

Items may declare named effect parameters:

```nova
pub type Retrying<effect E> {
    op: fn() -> Result<Bytes, NetError> uses [E]
    attempts: Int
}

pub fn twice<effect E>(f: fn() uses [E]) uses [E] {
    f()
    f()
}
```

- Declared with `effect Ident` in the generic parameter list.
- Used in `uses` clauses as a bare identifier.
- Passed explicitly as a generic argument with a `uses` clause: `Retrying<uses [network.http]>`. Usually inferred.
- **E0414:** an effect variable in a parameter type may appear only:
  - in the top-level row of a function-typed parameter, or
  - as an effect argument of a parameter's nominal type (`Tools<uses [E]>`).

  It may not appear nested deeper (e.g. inside a callback's parameter). This keeps inference first-order (§7.3).
- Types are **covariant** in their effect parameters, except that a `mut` value of such a type is **invariant** in them.

### 4.6 Implicit effect variables

Each `fn` item parameter whose type is a function type with an omitted row gets a fresh, anonymous effect variable. This is D16 from `build.md`.

```nova
pub fn map<T, U>(xs: List<T>, f: fn(T) -> U) -> List<U> { ... }
// ≡ pub fn map<T, U, effect ρ>(xs: List<T>, f: fn(T) -> U uses [ρ]) -> List<U> uses [ρ]
```

For `pub` functions, every implicit variable is automatically included in the declared row (§7.2). This holds whether or not the body calls the parameter: it's conservative but predictable.

---

## 5. Performing operations [MVP]

### 5.1 Namespaces

Effect names live in an **effect namespace**, separate from values, types, and modules.

- In effect positions (`uses`, `handle`, `handler`), only the effect namespace is consulted.
- In expression position, a dotted name `a.b…z(args)` resolves its first segment `a` in this order:
  1. local bindings
  2. items and modules in scope (including the prelude)
  3. effect roots in scope

   **W0402:** a local binding shadows an effect root that is used in the same function.
- **Merged module/effect names.** If `a` is a module *and* an effect root with the same name, the member `b` is looked up in the module first, then as an operation (§5.2). The standard library uses this on purpose:
  - module `fs` holds wrapper functions (`fs.read_text`)
  - effect `fs` holds operations (`fs.read`)

### 5.2 Operation resolution

For an expression `R.s₁.….sₙ(args)` where `R` resolved to effect root `R`:

1. Find the deepest node `N` reachable from `R` by following `s₁…sₖ` (k < n) as nested-effect names. Consume as many segments as possible, but always leave the last segment.
2. The remaining segments must be exactly one: the op name `sₙ`. Otherwise **E0404**.
3. Candidates are all ops named `sₙ` in `subtree(N)`.
   - Zero candidates: **E0404** unknown operation, with suggestions.
   - More than one: **E0403** ambiguous operation, listing fully qualified alternatives.
   - Exactly one: that op.

Examples, using built-ins from §9:

| Expression | Resolves to |
|---|---|
| `fs.read(path)` | `fs.read.read` — unique `read` under `fs` |
| `fs.read.list(path)` | `fs.read.list` |
| `network.http.request(req)` | `network.http.request` |
| `kv.delete(k)` | `kv.write.delete` |

Operations are values only when called. `let f = fs.read` is **E0417**; write `fn(p) { fs.read(p) }`.

### 5.3 Effect of a perform

Performing op `o` declared in leaf `L` has type `ret(o)` and contributes row `[L]`.

---

## 6. Handlers [MVP except where marked]

### 6.1 Named handlers

```ebnf
HandlerDecl = { DocComment } "handler" EffectPath "for" Type [ UsesClause ] "{" { OpImpl } "}"
OpImpl      = { DocComment } "fn" Ident "(" "self" { "," Param } [ "," ] ")" [ "->" Type ] Block
```

```nova
pub type MockKv { data: Map<String, Bytes> }

handler kv.read for MockKv {
    fn get(self, key: String) -> Result<Bytes?, KvError> { Ok(self.data.get(key)) }
    fn scan(self, prefix: String) -> Result<List<String>, KvError> {
        Ok(self.data.keys().filter(fn(k) { k.starts_with(prefix) }).to_list())
    }
}
```

Rules:

1. The handler handles node `N = EffectPath`. It must implement **every** op in `subtree(N)`.
   - Missing ops: **E0406**.
   - Extra ops: **E0404**.
   - Ops from different leaves with the same name: an interior-node handler for a subtree where an op name repeats is **E0403**. Handle the leaves separately.
2. Each op implementation must match the declared signature exactly: same parameter types in order, same return type. Parameter names may differ. Otherwise **E0407**.
3. `self` is the handler value (immutable unless the type is `mut`).
4. Only one `handler N for T` per `(N, T)` pair in the program (**E0418**). A type may handle several different nodes (resolves `build.md` §11 Q7).
5. The handler's row is the union of its op bodies' rows.
   - If `T` is `pub`, the `handler` declaration needs a `uses` clause bounding that union. An absent clause means `uses []` (pure).
   - Otherwise the row is inferred. Same rule as `pub fn` (§7.2).
6. Generic handler types are allowed: `handler fs.read for Under<T>`.

### 6.2 Inline handlers

```ebnf
HandleExpr     = "handle" HandlerBinding { "," HandlerBinding } Block
HandlerBinding = EffectPath "with" ( InlineHandler | Expr )
InlineHandler  = "{" { InlineOp } "}"
InlineOp       = "fn" Ident "(" [ InlineParam { "," InlineParam } [ "," ] ] ")" [ "->" Type ] Block
InlineParam    = Pattern [ ":" Type ]
```

- `with {` always begins an inline handler. To use a block expression as a handler value, parenthesize it: `with ({ … })`.
- Inline op parameter and return types may be omitted; they're taken from the op declaration. If written, they must match exactly (**E0407**).
- Inline op bodies may capture variables from the enclosing scope like closures do.

### 6.3 `handle` expressions

```nova
let cfg = handle fs.read with MockFs(files: files) {
    load_config("app.toml")?
}
```

- `handle E₁ with h₁, E₂ with h₂ { body }` is exactly `handle E₁ with h₁ { handle E₂ with h₂ { body } }`. So `h₂`'s op bodies that perform `E₁` reach `h₁`.
- `with` expressions are evaluated once, left to right, before `body`.
- The type of the `handle` expression is the type of `body`.
- The handler value for `E` must be of a type `T` with a declaration `handler E for T` (**E0419**: no such handler; the message lists the effects `T` does handle).

### 6.4 Typing a `handle`

```text
Γ ⊢ body : τ ! φ_b      H handles node N with row φ_H
───────────────────────────────────────────────────────────
Γ ⊢ handle N with H { body } : τ ! (φ_b ∖ leaves(N)) ∪ φ_H
```

- Subtraction removes leaves only. Effect variables are never removed: a handler for `fs` does not discharge an unknown `ρ`.
- `φ_H` is the handler's row (§6.1 rule 5, or inferred from inline op bodies) plus the row of evaluating the `with` expression.
- **W0403:** `N` is not in `φ_b`, so the handler is unused.

### 6.5 Delegation

Op bodies execute **outside** the `handle` they belong to (deep handlers, §8.2). So if a handler for `N` performs an op of `N`, the call goes to the next enclosing handler for `N`, and the typing rule above adds `N` back through `φ_H`. This is how wrappers are written:

```nova
fn logged_kv(prefix: String) -> LoggedKv { LoggedKv(prefix: prefix) }

handler kv.read for LoggedKv {
    fn get(self, key: String) -> Result<Bytes?, KvError> {
        console.print("{self.prefix} get {key}")
        kv.get(key)                 // delegates to the outer kv.read handler
    }
    fn scan(self, prefix: String) -> Result<List<String>, KvError> { kv.scan(prefix) }
}
// row of this handler: [console, kv.read]
```

### 6.6 Tail resumption [MVP]

The value an op body evaluates to (or `return`s) becomes the result of the `perform`, and execution continues after it. This is the only resumption mode in the MVP.

### 6.7 `abort` [MVP]

```ebnf
AbortExpr = "abort" Expr
```

- Allowed only lexically inside an **inline** handler's op body, and not inside a nested lambda or `fn` (**E0409**).
- Its target is the `handle` expression the inline handler belongs to.
- `abort e` has type `Never`. `e`'s type must equal the target `handle` expression's type (**E0420**).
- Semantics: unwind everything between the perform and the target `handle`, then make `e` the target's value.
  - Unwinding runs no user code (Nova has no destructors or `finally` in v0).
  - Values held by unwound frames are dropped.

Named handlers cannot `abort`. To deny an operation, they return `Err` (see `deny`, §9.2) or panic.

### 6.8 General `resume` [P7]

```ebnf
ResumeExpr = "resume" "(" Expr ")"
```

- Allowed only inside op bodies.
- If an op body contains `resume`, its result is the result of the whole `handle` expression rather than the perform's value. `resume(v)` continues the suspended computation with `v` and evaluates to that continuation's final result.
- A continuation can be resumed at most once. A second `resume` panics with `ResumedTwice`.
- An op body that finishes without resuming behaves like `abort`.
- **E0410** until Phase 7: "`resume` is not supported yet; return a value instead (tail resumption)".

### 6.9 Return clauses [Future]

A handler clause that transforms the body's final value (Koka's `return`) is reserved and not specified.

---

## 7. Inference and checking [MVP]

### 7.1 Body rows

The row of an expression is the union of its subexpressions' rows, plus:

| Construct | Adds |
|---|---|
| Perform op in leaf `L` | `[L]` |
| Call `f(args)` where `f : fn(…) -> R uses φ` | `φ` (after instantiation, §7.3) |
| Method call | the method's row (after instantiation) |
| Trait method call on a generic `T: Tr` | the trait method's declared bound (§7.5) |
| Lambda expression `fn(x) { body }` | nothing; `body`'s row becomes the lambda's *type* row |
| `handle` | per §6.4 |
| `abort e` | row of `e` |

### 7.2 Declared vs. inferred

1. **Private functions** (no `pub`):
   - Without a `uses` clause, their row is inferred.
   - With a clause, the inferred row must satisfy `inferred ⊑ declared` (**E0402**), and callers see the *declared* row.
2. **`pub` functions** must have their row declared. An absent clause is `uses []`. The allowed row is `declared ∪ {implicit variables of parameters}`.
   - The body must satisfy `inferred ⊑ allowed` (**E0402**).
   - **W0401:** a declared leaf the body never uses. Over-declaring is legitimate for API stability; suppress with `@allow(unused_effects)`.
3. `tool fn` and `eval` items follow the same rules; see `ai.md`.
4. **Recursion.** Private functions are grouped into strongly connected components of the call graph and solved by fixpoint:
   - start every row in the group empty
   - recompute every body's row from the current rows
   - repeat until nothing changes

   This terminates because rows only grow and the universe of leaves is finite. A component containing a `pub` function uses that function's declared row as a fixed point.

### 7.3 Instantiation at call sites

When calling `f<…, effect E₁…Eₖ>(args)` (implicit or named variables):

1. Each `Eᵢ` gets a fresh instance `ρᵢ`.
2. For each argument at a parameter position where `Eᵢ` appears (always a top-level fn row or an effect argument, per E0414), the argument's row joins `ρᵢ`. That is, `ρᵢ := ⋃ rows(argument types at those positions)`.
   - Lambda arguments are checked *against* the expected parameter type. Their body row is inferred first, then contributed.
3. The call's row is the callee's row with each `Eᵢ` replaced by `ρᵢ`.
4. Explicit effect arguments (`twice<uses [console]>(…)`) fix `ρᵢ`. Arguments must then satisfy `row ⊑ ρᵢ` (**E0412**).

Since effect variables appear only in first-order positions, step 2 is a single pass. There is no unification of rows.

Inside a function body, its own effect variables are **rigid**. Calling `f: fn() uses [ρ]` adds the variable `ρ` itself, and `ρ` is only allowed by the declaration that introduced it.

### 7.4 Restricting callbacks

```nova
pub fn memoize<K: Hash, V>(f: fn(K) -> V uses []) -> Memo<K, V> uses [] { ... }

memoize(fn(k) { fetch(k) })
// error[E0412]: closure uses `network.http` but `memoize` requires `uses []`
```

### 7.5 Traits

- A trait method's `uses` clause is an **upper bound** for all impls. Absent means pure.
- An impl method's row must satisfy `impl_row ⊑ trait_bound` (**E0411**).
- Calls through a generic bound `T: Tr` add the trait bound.
- Calls on a concrete type add the impl's actual row.

### 7.6 The root check

For `fn main() [-> Result<(), E>] uses φ_main`:

1. The body must satisfy `inferred ⊑ φ_main` (**E0402**).
2. Every leaf in `φ_main` must have a runtime root handler. Leaves of user-defined effects never do (**E0408**: "`kv` cannot be provided by the runtime; handle it in `main`, e.g. `handle kv with SqliteKv.open(…)?`").
3. `φ_main` may not contain variables or `ffi` (**E0408**, **E0413**).

For `test` and `eval` items, the row is inferred (they have no declaration). Rule 2 applies with the test/eval root-handler set from §8.4.

---

## 8. Runtime semantics [MVP]

### 8.1 Handler stack

Each executing task (the MVP has one) has a **handler stack**: an ordered list of frames `(node N, handler value h, handle_id)`.

- `handle N with h { body }` pushes a frame, evaluates `body`, and pops the frame on normal completion, `abort`, or panic unwinding.
- Root handlers sit at the bottom of the stack, pushed before the entry point runs.

### 8.2 Dispatch

Performing op `o` of leaf `L`:

1. Search the stack from top to bottom for the first frame whose node `N` satisfies `L ∈ leaves(N)`. The type system guarantees one exists; if none does, that's an internal compiler error.
2. Let the found frame be at index `i`. Evaluate the op body for `o` in `h` with the **visible stack truncated to frames `[0, i)`**. Frames at `i` and above are hidden while the op body runs; this gives delegation.
3. When the op body produces a value `v`, restore the full stack and continue after the perform with `v` (tail resumption).
4. If the op body executes `abort e` targeting `handle_id`:
   - unwind to that `handle`'s frame
   - pop it
   - the `handle` expression evaluates to `e`

### 8.3 Dynamic scope and escaping closures

Handlers are **dynamically scoped**. A closure created inside `handle kv with A { … }` that escapes and is called later dispatches to whichever `kv` handler is installed *at call time*. The type system still guarantees one exists, because the closure's type row includes `kv.*`. This matches OCaml 5 and Koka.

Code that needs to capture a specific handler should pass a handler value and wrap its calls in `handle` explicitly.

### 8.4 Root handlers

| Entry point | Root handlers installed |
|---|---|
| `nova run` (`main`) | Real handlers for exactly the built-in leaves in `φ_main`, minus any denied by `--allow` (§9.2, §10.2). |
| `nova test` (`test` items) | Real handlers for all built-in leaves, except `ai.*` → replay handler and `approval` → auto-approve (`ai.md` §13.2, §10.4). Resolves `build.md` §11 Q8. |
| `nova eval` (`eval` items) | Real handlers for all built-in leaves, including live `ai.*`; `approval` → auto-deny unless `--approve`. |

### 8.5 Panics inside handlers

A panic in an op body unwinds through the hidden frames and then through the perform site, exactly as if the perform had panicked.

---

## 9. Built-in effects [MVP]

### 9.1 Declarations

These live in `std/effects.nova`, are `pub`, and are in the prelude. Wrapper functions with friendlier names live in the same-named std modules (§5.1).

```nova
pub effect console {
    fn print(text: String)
    fn eprint(text: String)
    fn read_line() -> String?
}

pub effect fs {
    effect read {
        fn read(path: Path) -> Result<Bytes, IoError>
        fn list(path: Path) -> Result<List<Path>, IoError>
        fn metadata(path: Path) -> Result<Metadata, IoError>
    }
    effect write {
        fn write(path: Path, data: Bytes) -> Result<(), IoError>
        fn append(path: Path, data: Bytes) -> Result<(), IoError>
        fn create_dir(path: Path) -> Result<(), IoError>
        fn delete(path: Path) -> Result<(), IoError>
        fn rename(from: Path, to: Path) -> Result<(), IoError>
    }
}

pub effect network {
    effect http {
        fn request(req: HttpRequest) -> Result<HttpResponse, NetError>
    }
}

pub effect process {
    effect exec {
        fn run(cmd: Command) -> Result<Output, ProcError>
    }
    effect env {
        fn var(name: String) -> String?
        fn args() -> List<String>
    }
}

pub effect time {
    effect clock {
        fn now() -> Instant          // wall clock
        fn monotonic() -> Duration   // since an arbitrary epoch
    }
    effect sleep {
        fn sleep(d: Duration)
    }
}

pub effect random {
    fn next_u64() -> Int
    fn fill(n: Int) -> Bytes
}
```

- `ai` and `approval` are declared in `ai.md` §3 and §10.4.
- `network.tcp` is post-MVP.
- `IoError`, `NetError`, and `ProcError` each include `Denied(reason: String)`.

### 9.2 Standard handlers

Defined in std, `pub`:

| Handler | Handles | Behavior |
|---|---|---|
| `deny` | any built-in node | Built-in special form: `handle fs.write with deny { … }`. Ops returning `Result<_, E>` with `E: Deniable` return `Err(E.Denied("<path> denied"))`. Other ops panic with `SandboxViolation(<path>)`. Row `[]`. |
| `fs.read.under(root)` | `fs.read` | Paths are canonicalized; anything outside `root` → `Err(IoError.Denied)`. Delegates otherwise. Row `[fs.read]`. |
| `fs.under(root)` | `fs` | Same, for both read and write. |
| `network.http.only(hosts)` | `network.http` | Host allowlist; delegates. |
| `process.exec.only(programs)` | `process.exec` | Program allowlist; delegates. |
| `time.fixed(instant)` | `time.clock` | Deterministic clock for tests. |
| `random.seeded(seed)` | `random` | Deterministic PRNG. |

### 9.3 `ffi`

- `ffi` is a reserved root with no operations, and `φ ⊑ [ffi]` for every row.
- Any future FFI or unsafe construct performs `ffi`.
- `ffi` can never appear in `--allow` (**E0413**) and has no root handler in the MVP.
- No MVP construct performs it.

---

## 10. Sandboxing and packages [MVP]

### 10.1 Static check

`nova check --allow <paths> <target>`:

- Computes the declared row of every `pub` fn, `tool fn`, and `pub` handler in `<target>`, with effect variables removed.
- Fails with **E0421** for each item whose row is not `⊑` the allow-list. The diagnostic gives the item, the excess leaves, and a provenance chain.

### 10.2 Dynamic enforcement

`nova run --allow <paths>`:

1. The static check runs first on `main`.
2. The runtime installs root handlers as in §8.4, but substitutes `deny` for every built-in leaf not in the allow-list.

This layer matters for modules that are loaded dynamically, and as defense in depth.

### 10.3 Package effect sets

- A package's **effect set** is the compacted union of the declared rows of all its `pub` items, with effect variables removed.
- `nova.lock` records it per package (resolves `build.md` §11 Q9 as "union per package" for v0.1; per-item detail is available from `nova why-effect`).
- `nova update` compares the old and new sets. Any added leaf, including leaves that appear because a parent gained a child, fails with **E0422**, unless `--accept-effects` is passed. The diff is printed either way.

---

## 11. Documented exceptions

These are visible behaviors outside the effect system. Each is deliberate.

1. **Panics and non-termination** are not tracked.
2. **Memory allocation** is not tracked.
3. **Runtime AI handlers** (budget, trace, cache, record, replay) read the monotonic clock and write trace/cache files internally. They're implemented by the runtime, configured from the CLI and `nova.toml`, and don't add `time.clock` or `fs.write` to user rows. See `ai.md` §12.
4. **Diagnostics output** from the runtime itself (panic messages, `--trace` summaries) goes to stderr without `console`.

---

## 12. Diagnostics

| Code | Level | Meaning |
|---|---|---|
| E0401 | error | Unknown effect path |
| E0402 | error | Row exceeds declared `uses`. Must show a provenance chain (§12.1) |
| E0403 | error | Ambiguous operation name |
| E0404 | error | Unknown operation, or extra op in a handler |
| E0405 | error | Effect node mixes ops and sub-effects, or is empty |
| E0406 | error | Handler missing operations |
| E0407 | error | Op implementation signature mismatch |
| E0408 | error | Effect cannot be provided at root |
| E0409 | error | `abort` outside an inline handler op body |
| E0410 | error | `resume` not supported yet (until P7) |
| E0411 | error | Impl method row exceeds trait bound |
| E0412 | error | Argument row exceeds parameter's row |
| E0413 | error | `ffi` cannot be granted |
| E0414 | error | Effect variable in an unsupported position |
| E0415 | error | Generic operation |
| E0416 | error | Duplicate operation in a leaf |
| E0417 | error | Operation used as a value |
| E0418 | error | Duplicate handler declaration for `(node, type)` |
| E0419 | error | Type does not handle this effect |
| E0420 | error | `abort` value type mismatch |
| E0421 | error | Item exceeds `--allow` |
| E0422 | error | Dependency update adds effects |
| W0401 | warning | Declared effect is never used |
| W0402 | warning | Local binding shadows an effect root |
| W0403 | warning | Handler for an effect the body doesn't perform |

### 12.1 Provenance

The checker records, for each leaf in each inferred row, the **first** expression in source order that introduced it, plus the callee through which it arrived. E0402, E0412, E0421, and `nova why-effect` render the chain by following callees, up to 8 hops, then `…`:

```text
error[E0402]: `summarize` uses `process.exec` but declares `uses [ai]`
  --> src/summary.nova:12:5
   |
 3 | pub fn summarize(doc: Document) -> Result<Summary, AiError> uses [ai] {
   |                                                             --------- declared here
12 |     clean(doc.text)
   |     ^^^^^^^^^^^^^^^ `process.exec` enters here
   |
   = note: clean (src/text.nova:40) → run_formatter (src/text.nova:55) → process.exec.run
   = help: add `process.exec` to `uses`, or handle it: `handle process.exec with … { … }`
```

If a leaf entered through an implicit effect variable, the chain names the argument: "via closure argument to `map` at src/x.nova:9".

---

## 13. Implementation notes (non-normative)

- **Leaf interning.** Intern all leaf paths into dense IDs at load time. Represent rows as `(BitSet<LeafId>, SmallVec<EffectVarId>)`. The number of leaves is small: dozens.
- **Provenance** is a side table: `HashMap<(FnId, LeafId), Origin { span, via: Option<FnId> }>`. It's built during inference and only consulted for diagnostics.
- **SCCs:** Tarjan over the private call graph, per module.
- **Interpreter handler stack:** `Vec<Frame>` plus a "visible top" index. A perform:
  - scans from the visible top downward
  - saves the current top
  - sets the top to `i` while the op body runs, then restores it
- **`abort`:** propagate `Control::Abort { handle_id, value }` through the evaluator's `Result`-like return type. The `handle` evaluation matches its own ID.
- **Root handlers:** `nova_runtime` implements them as native Rust handler objects behind the same dispatch interface as Nova-defined handlers.

---

## 14. Conformance tests (minimum)

Each bullet becomes at least one file under `tests/ui/effects/` or `tests/run/effects/`.

1. Hierarchy normalization and compaction in diagnostics (`fs.read, fs.write` displays as `fs`).
2. Op resolution: unique short form, qualified form, ambiguity (E0403), unknown (E0404), op used as a value (E0417).
3. Module/effect name merging: `fs.read_text` (module) vs. `fs.read` (op).
4. `pub` without `uses` is pure; E0402 with a 3-hop provenance chain.
5. Private inference, including mutual recursion across 3 functions.
6. Implicit variables: `map` with a pure lambda (row `[]`) vs. an effectful one; nested callback position gives E0414.
7. Callback restriction (E0412).
8. Trait bound violation (E0411).
9. Named handler: missing op (E0406), signature mismatch (E0407), unused handler (W0403).
10. Inline handler with omitted parameter types; `abort` type mismatch (E0420); `abort` in a lambda (E0409).
11. Delegation: a logging wrapper over a mock; output order verified at runtime.
12. Multi-binding `handle` order: inner op delegates to outer.
13. Escaping closure dispatches to the handler installed at call time.
14. Root: a user effect in `main` without a handler (E0408); `ffi` in `--allow` (E0413).
15. `deny` on a Result op (returns `Err`) and on a non-Result op (panics with `SandboxViolation`).
16. `--allow` static failure (E0421) and dynamic substitution.
17. Lockfile effect diff refusal (E0422).
18. `resume` gives E0410 in the MVP.
