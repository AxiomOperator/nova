# Nova Build Document

Status: draft v0.3 — 2026-10-04
Supersedes `docs/plan.md` wherever the two conflict. `plan.md` remains the vision essay; this document is the engineering plan.

---

## 1. Thesis

**Nova is a statically typed language with first-class AI, whose functions declare what they are allowed to do, and whose compiler enforces it.**

Two pillars, designed together:

1. **Effects.** Effects (`uses [network]`) are part of every function's type. They are inferred inside a module, required at public boundaries, recorded in package lockfiles, and implemented by swappable **handlers**.
2. **First-class AI.** Model inference is a built-in effect. The compiler derives a schema for any data type, understands prompt literals, model declarations, tool functions, and eval blocks, and checks agents' permissions statically.

Each pillar makes the other stronger:

- **Guardrails for AI-generated code.** A function declared `uses [ai]` cannot call `process.exec`; the compiler rejects it.
- **Guardrails for AI agents.** An agent's tools are ordinary functions with known effects. An agent can never do more than its caller is allowed to do, and that is checked at compile time.
- **Testing without DI frameworks.** Swap the real `db`, `network`, or `ai` handler for a mock, a recording, or a replay.
- **Sandboxing.** Run a module or agent with a restricted effect set. Violations are rejected statically, and runtime handlers enforce the boundary.
- **Supply-chain visibility.** `nova update` flags a dependency that newly gains `network`, `ai`, or `process.exec`.
- **Operability.** Every AI call is typed, budgeted, traced, and replayable.

HTTP servers, SQL, and other domain features are libraries built on effects plus compile-time reflection. AI is not a library: it is part of the language.

### Goals (MVP, v0.1)

1. A type + effect checker with good error messages. When a function uses an effect it didn't declare, the error shows the call chain by which the effect entered.
2. A working interpreter with effect handlers, enough to run real scripts.
3. First-class AI:
   - typed generation and streaming, and embeddings
   - model declarations
   - tool functions and effect-bounded agents
   - budgets and tracing
   - record/replay testing and evals
4. Six demos (§8).

### Non-goals (MVP)

- Native code generation, GC, performance work.
- General concurrency (tasks, actors, supervision). Agent tool calls run sequentially.
- Package registry; only local/path and git dependencies.
- LSP, debugger, REPL polish.
- `api`, query, `agent`, or `memory` keywords. Agents are typed std values built on core primitives (§5.7).
- Structural typing, manual memory management, FFI beyond the interpreter's built-ins.
- Training, fine-tuning, or running models in-process. Nova calls model servers; it does not host them.
- Self-hosting.

---

## 2. Decision log

Decisions made 2026-10-04. Changing one requires a new entry in `docs/decisions/` with rationale.

| # | Topic | Decision |
|---|---|---|
| D1 | Purpose | Serious language aimed at real users. |
| D2 | Core differentiator | Effect/capability system **and** first-class AI, co-equal. |
| D3 | First milestone | Parser + type/effect checker + tree-walking interpreter. |
| D4 | Team | Solo developer directing AI agents. The plan must break into small, spec-referenced, test-first tasks. |
| D5 | Effect annotations | Inferred for private functions; **required on `pub` functions**. |
| D6 | Capability delivery | Effects + handlers. Effect operations are invoked by name; the nearest enclosing handler implements them. |
| D7 | Granularity | Hierarchical effects (`fs` ⊇ `fs.read`, `fs.write`); user-definable. |
| D8 | Supply chain | Manifests/lockfiles record each dependency's effect set; updates warn on new effects. Core feature. |
| D9 | Mutability | Immutable by default. `let` immutable, `var` rebindable, `mut` for mutable objects. |
| D10 | Concurrency safety | Isolated tasks/actors; only immutable or `Send` data crosses boundaries (post-MVP). |
| D11 | Errors | `Result<T, E>` for recoverable errors; panics for bugs, which crash the current task/actor. |
| D12 | Types | Nominal structs/enums, traits, generics with trait bounds. Structural records possibly later. |
| D13 | Batteries | **AI is core**: built-in `ai` effect, compiler-known `Schema`, `prompt` literals, `model` items, `tool` functions, `eval` blocks. Model providers are handlers, so provider churn stays out of the compiler. HTTP/SQL are libraries built on reflection. |
| D14 | Backend | Interpreter first → Cranelift for dev builds → LLVM for release builds. |
| D15 | Handlers | One-shot resumable (OCaml 5 style). MVP implements the tail-resumptive + abortive subset (§4.6). |
| D16 | Effect polymorphism | Implicit row variables; callback effects flow into the caller automatically. |
| D17 | Implementation language | Rust. |
| D18 | Syntax | No semicolons; `T?` sugar for `Option<T>`; expression-oriented; named arguments. |
| D19 | Docs | Markdown in repo. `docs/spec/` is the language source of truth. |
| D20 | AI scope | MVP includes tools, agents, streaming, embeddings, budgets, and observability. |
| D21 | AI reliability | Record/replay handler for deterministic tests; built-in `eval` blocks and `nova eval`. |
| D22 | AI timing | AI is co-equal in the MVP; AI work starts in Phase 3 alongside the interpreter. |
| D23 | Agents | Agents are std values built on core `tool`/`model`/`ai` primitives, with compiler-checked effect bounds. No `agent` keyword in the MVP; revisit after Demo 5 (§11, Q11). |

---

## 3. Language core v0

This is a sketch, not the spec. Each subsection becomes a file in `docs/spec/` during Phases 0–2. Syntax not shown here is undecided (see §11).

### 3.1 Lexical

- Newline-terminated statements. A line continues if it ends in a binary operator, `(`, `[`, `{`, `,`, or `->`, or if the next line starts with `.`.
- Comments: `//` line, `///` doc. Doc comments are semantic for AI: they become schema and tool descriptions (§5.3, §5.6). No block comments in v0.
- String literals with interpolation: `"hello {name}"`. Triple-quoted `"""…"""` for multiline strings (dedented).
- Prompt literals: `prompt"…"` and `prompt"""…"""` (§5.4).

### 3.2 Bindings and mutability

```nova
let xs = [1, 2, 3]       // immutable binding, immutable value
var count = 0            // rebindable
count += 1
let buf = mut Buffer()   // binding is fixed; the object is mutable
buf.push(1)
```

- Values are immutable unless created with `mut`. A `mut T` is a distinct type from `T`; it coerces to `T` (a read-only view) but not vice versa.
- Only immutable values will be `Send` (D10).

### 3.3 Types

```nova
type User {
    id: UUID
    name: String
    active: Bool = true
}

enum Shape {
    Circle(radius: Float)
    Rect(w: Float, h: Float)
    Empty
}

trait Describe {
    fn describe(self) -> String
}

impl Describe for User {
    fn describe(self) -> String { "user {self.name}" }
}

fn largest<T: Ord>(xs: List<T>) -> T? { ... }
```

- Built-ins: `Int` (64-bit), `Float` (64-bit), `Bool`, `String` (UTF-8), `Bytes`, `List<T>`, `Map<K, V>`, `Set<T>`, `Option<T>` / `T?`, `Result<T, E>`, `()`.
- AI built-ins: `Prompt`, `Model`, `Tool`, `Vector`, `Stream<T>`, `Partial<T>` (§5).
- Local type inference. Function signatures require parameter and return types.
- `match` must be exhaustive.
- `if let Some(u) = maybe_user { ... }`.
- Error types are ordinary enums. There is no separate `error` keyword.

### 3.4 Functions and expressions

```nova
fn area(s: Shape) -> Float {
    match s {
        Circle(r)  => 3.14159 * r * r
        Rect(w, h) => w * h
        Empty      => 0.0
    }
}

retry(times: 3, after: 2.seconds)
```

- The last expression of a block is its value; `return` exits early.
- Named arguments are allowed for any parameter. Positional arguments must come before named ones.
- Lambdas: `fn(x) { x + 1 }`. A short form is undecided.

### 3.5 Errors and panics

- `?` on `Result<T, E>` returns `Err(e.into())` early; on `T?` it returns `None` early.
- `panic("msg")` aborts the current task (in the MVP, the program). Panics are not effects and cannot be caught except at a task/actor boundary.
- Out-of-bounds indexing, integer overflow, and unwrapping `None` panic. Non-panicking forms: `xs.get(i) -> T?`, `checked_add`.
- **Every AI operation returns `Result`.** Model calls fail often (network, rate limits, invalid output, budget), so none of them panic.

### 3.6 Auto traits and reflection

- **`Schema` and `Json` are auto traits.** Any type built only from `Schema` types is `Schema` automatically: primitives, collections, `Option`, structs, enums. The same goes for `Json`. No `derive` is needed.
  - This is what makes `ai.generate<T>` work for any plain data type.
  - Opt out with `@opaque`. Types holding functions, handlers, or `mut` state are never `Schema`.
- Field and variant doc comments (`///`) and validation attributes (§5.3) become part of the generated schema.
- Explicit derives in the MVP: `Eq`, `Ord`, `Hash`, `Debug`.
- General compile-time reflection (user-written derives) is post-MVP.

---

## 4. The effect system

> Normative details: `docs/spec/effects.md`. This section is an overview; the spec wins where they differ.

### 4.1 Declaring effects

An effect is a named set of operations, like a trait without a receiver.

```nova
effect console {
    fn print(s: String)
    fn read_line() -> String?
}

effect fs {
    effect read {
        fn read(path: Path) -> Result<Bytes, IoError>
        fn list(path: Path) -> Result<List<Path>, IoError>
    }
    effect write {
        fn write(path: Path, data: Bytes) -> Result<(), IoError>
        fn delete(path: Path) -> Result<(), IoError>
    }
}
```

- Nested `effect` blocks create a hierarchy: `fs` means `fs.read ∪ fs.write`.
- Invoking an operation adds its leaf effect (e.g. `fs.read`) to the caller's effect set. Calls name the operation through any prefix of its path, as long as the name is unique in that subtree: `fs.read(path)` and `fs.read.read(path)` are equivalent (`effects.md` §5).
- Effect operations may be generic (`ai.generate<T>`).

### 4.2 Built-in effects (MVP)

| Effect | Operations (illustrative) |
|---|---|
| `console` | `print`, `eprint`, `read_line` |
| `fs.read`, `fs.write` | `read`, `list`, `write`, `delete` |
| `network.http` | `request` |
| `process.exec` | `run` |
| `process.env` | `get`, `args` |
| `time.clock` | `now` |
| `time.sleep` | `sleep` |
| `random` | `int`, `float`, `bytes` |
| `ai.generate` | `generate<T>`, `stream<T>` (§5.2) |
| `ai.embed` | `embed` (§5.9) |
| `approval` | `approve` — human-in-the-loop confirmation (§5.7) |

`db` is **not** built in. It is library-defined, which proves user-defined effects are first class.

Panicking and non-termination are not tracked.

### 4.3 Signatures, inference, and the `pub` rule (D5)

```nova
fn helper(url: Url) -> Result<Bytes, NetError> {     // inferred: uses [network.http]
    network.http.request(Request.get(url))?.body()
}

pub fn fetch_config(url: Url) -> Result<Config, Error>
    uses [network.http]                               // required: pub
{
    Config.parse(helper(url)?)
}
```

- Private functions: effects are inferred. If written, `uses [...]` is an upper bound that the checker verifies.
- `pub` functions **must** declare `uses [...]`. A `pub fn` with no clause means `uses []` (pure).
- Declaring a parent effect (`uses [fs]`) permits any child (`fs.read`).
- Inference is per module. Mutually recursive private functions are solved together, by iterating to a fixpoint.
- `main` declares the program's full capability set.

### 4.4 Effect polymorphism (D16)

Function types carry an effect row. A row that isn't written is an implicit row variable.

```nova
pub fn map<T, U>(xs: List<T>, f: fn(T) -> U) -> List<U> { ... }
// Internally: map<T, U, ε>(xs, f: fn(T) -> U uses ε) -> List<U> uses ε

let pages = urls.map(fn(u) { fetch(u) })   // this call: uses [network.http]
```

- On `pub` functions, an unannotated function-typed parameter contributes its row to the function's effects. That is the only implicit row allowed at a `pub` boundary.
- To *restrict* a callback, annotate it: `f: fn(T) -> U uses []`.
- Named effect parameters (`fn twice<effect E>(f: fn() uses [E]) uses [E]`) are available where implicit rows aren't enough, e.g. `Tools<uses [E]>` and `Agent<E>` (`effects.md` §4.5).

### 4.5 Handlers (D6, D15)

A handler gives meaning to an effect's operations for the duration of a block.

```nova
type MockFs { files: Map<Path, Bytes> }

handler fs.read for MockFs {
    fn read(self, path: Path) -> Result<Bytes, IoError> {
        self.files.get(path).ok_or(IoError.NotFound(path))
    }
    fn list(self, path: Path) -> Result<List<Path>, IoError> { ... }
}

test "loads config" {
    handle fs.read with MockFs(files: {"app.toml": b"port = 80"}) {
        assert_eq(load_config("app.toml")?.port, 80)
    }
}
```

```nova
handle process.exec with {
    fn run(cmd: Command) -> Result<Output, ProcError> {
        abort Err(SandboxError.Denied("process.exec"))
    }
} {
    untrusted.main()
}
```

Semantics:

- `handle E with H { body }` removes `E` (and its children, if `H` handles the parent) from `body`'s effect set. The handler's own effects are added instead.
- **Delegation:** inside a handler for `E`, performing `E` reaches the *next outer* handler. This is how wrappers are built: budgets, tracing, caching, recording, narrowing.
- **Tail-resumptive** handlers return a value from the operation. This is the common case: mocks, real implementations, wrappers.
- **Abortive:** `abort expr` ends the `handle` block with `expr` as its value.
- **General one-shot `resume(v)`** (continuing from a non-tail position) is specified now, implemented post-MVP (Phase 7).
- Calling `resume` twice panics.
- The runtime installs root handlers for exactly the effects `main` declares. An unhandled effect at a program root is a compile error.

### 4.6 What the MVP implements

| Feature | MVP | Post-MVP |
|---|---|---|
| Effect declaration, hierarchy, inference, `pub` rule | ✅ | |
| Implicit row polymorphism | ✅ | explicit row syntax |
| Tail-resumptive handlers, delegation, `abort` | ✅ | |
| Non-tail one-shot `resume` | ❌ | Phase 7 (bytecode VM) |
| Effects in lockfile | ✅ (path/git deps) | registry |

Why this split: tail-resumptive handlers are function calls, and `abort` is an unwind. Both are easy in a tree-walker. Streaming (§5.8) fits the subset because a stream is a pull-based value returned by the handler.

### 4.7 Sandboxing

There are two layers. Both are needed.

1. **Static:** `nova check --allow console,fs.read path/to/module` fails if any `pub` entry point's effects aren't a subset of the allowed set.
2. **Dynamic:** `nova run --allow console,fs.read` installs root handlers only for the allowed effects. Denied effects get handlers that `abort` with `SandboxError`.

Handlers can *narrow* capabilities instead of just allowing or denying them. Examples: an `fs.read` handler limited to `./data`, or an `ai` handler limited to a single local model. This replaces parameterized effects in the type system.

**Escape hatches must be effects.** Any future FFI or `unsafe` construct carries the `ffi` effect, which is a superset of everything and can never be granted by a sandbox.

### 4.8 Packages and effects (D8)

```toml
# nova.toml
[project]
name = "weather"
version = "0.1.0"

[dependencies]
http_client = { git = "https://…", tag = "v0.3.0" }

[models]                     # overrides for `model` items (§5.5)
fast = { provider = "openai_compatible", url = "http://localhost:8000/v1", name = "qwen3-8b" }
```

```toml
# nova.lock (generated)
[[package]]
name = "http_client"
version = "0.3.0"
source = "git+https://…#abc123"
effects = ["network.http", "time.clock"]
```

- `nova update` prints a diff of effect sets and **refuses** updates that gain effects unless run with `--accept-effects`.
- `nova why-effect process.exec` shows which dependency and call chain introduced an effect.

---

## 5. First-class AI

> Normative details: `docs/spec/ai.md`. This section is an overview; the spec wins where they differ.

### 5.1 Principles

1. **AI is an effect.** Code that calls a model says so in its type (`uses [ai]`). Nondeterminism is visible and controllable.
2. **Typed in, typed out.** Requests are built from `Prompt` values. Responses are validated into Nova types. Nobody writes `response["choices"][0]`.
3. **Providers are handlers.** The compiler knows the `ai` effect, `Schema`, `Prompt`, `model`, `tool`, and `eval`. It knows no vendor. OpenAI-compatible (vLLM, Ollama, llama.cpp), Anthropic, and other providers ship in std as handlers.
4. **Agents can't exceed their caller.** An agent's capabilities are the union of its tools' effects, checked at compile time against the caller's `uses`.
5. **Every call is operable.** Budgets, tracing, retries, caching, recording, and replay are handler wrappers, all built in.

### 5.2 The `ai` effect

Operations are untyped, because effect operations can't be generic (`effects.md` §3.2). Typing happens in std wrapper functions. The full types and the complete `AiError` enum are in `docs/spec/ai.md` §3.

```nova
// std/effects.nova (built-in declaration)
pub effect ai {
    effect generate {
        fn complete(req: Request) -> Result<Response, AiError>
        fn complete_stream(req: Request) -> Result<EventStream, AiError>
    }
    effect embed {
        fn embed_batch(req: EmbedRequest) -> Result<EmbedResponse, AiError>
    }
}
```

The user-facing API is the typed wrappers `ai.generate<T>`, `ai.stream<T>`, and `ai.embed`, with named arguments:

```nova
let s = ai.generate<Sentiment>(
    system: "You are a precise sentiment classifier.",
    prompt: prompt"Classify: {text}",
    model: fast,          // optional; defaults to the handler's default model
    temperature: 0.0,
    retries: 2,
)?
```

### 5.3 Schemas

```nova
/// Sentiment of a piece of text.
type Sentiment {
    /// Overall polarity.
    label: Label
    /// Model's confidence, 0 to 1.
    @range(0.0, 1.0)
    confidence: Float
    /// Short justification.
    @len(max: 280)
    reason: String
}

enum Label { Positive, Neutral, Negative }
```

- `Schema` is automatic (§3.6). `ai.generate<Sentiment>` sends a JSON Schema derived from the type, including doc comments as `description` and attributes as constraints. Providers that support it receive it as a structured-output constraint.
- Responses are parsed, then validated against the type and its attributes. Validation attributes: `@range`, `@len`, `@pattern`, `@one_of`, `@format(email | url | date)`.
- If validation fails, the request is retried up to `retries` times, with the validation errors fed back to the model. After that it returns `Err(InvalidOutput(raw, errors))`.
- Enums map to `enum`/`oneOf`; `Option<T>` maps to nullable; `Map<String, T>` maps to `additionalProperties`.

### 5.4 Prompts

```nova
let p = prompt"""
    Summarize the document for a {audience} reader.
    Document:
    {doc.text}
    """
```

- `prompt"…"` produces a `Prompt`, not a `String`. Interpolated values become **typed, delimited slots**: strings are fenced and labeled as data, and `Json` values are serialized.
- `Prompt` values compose: `p + prompt"Also list three key points."`, and `Prompt.join(parts)`.
- Traces record the template and the slot values separately, which makes prompts diffable and redactable.
- **This mitigates, but does not solve, prompt injection.** The guardrails that actually matter are effect bounds on tools and agents (§5.7). The docs must say so plainly.

### 5.5 Models

```nova
model fast  = openai_compatible(url: "http://localhost:8000/v1", name: "qwen3-8b")
model smart = anthropic(name: "claude-sonnet-5-5")
model embedder = openai_compatible(url: "http://localhost:8000/v1", name: "bge-m3")

model default = fast.fallback(smart)
```

- `model` is a top-level item that names a provider configuration, of type `Model`. It is resolved at program start.
- Overrides apply in this order: environment `NOVA_MODEL_<NAME>_*` > `nova.toml [models]` > source. Secrets (API keys) come only from the environment or `nova.toml` secret references, never from source.
- Combinators: `.fallback(other)`, `.with(temperature: …, max_tokens: …)`, `.retry(times:, backoff:)`.
- If `main` declares `uses [ai]`, the runtime installs a root `ai` handler that routes each request to its `model` (or to `default`). The network access belongs to that root handler.
  - `nova check --effects` reports it as `ai → network.http (via model providers)`.
  - `nova run --allow ai --models fast` restricts which models are reachable.
- Std providers in the MVP: `openai_compatible`, `openai`, and `anthropic`. Mocking is done with handlers (§5.11), not a provider.

### 5.6 Tools

```nova
/// Look up the current weather for a city.
tool fn weather(
    /// City name, e.g. "Austin".
    city: String,
) -> Result<Weather, WeatherError> uses [network.http] {
    ...
}

/// Read a UTF-8 file under the project directory.
tool fn read_file(path: Path) -> Result<String, IoError> uses [fs.read] { ... }

/// Delete a file.
@confirm
tool fn delete_file(path: Path) -> Result<(), IoError> uses [fs.write] { ... }
```

- `tool fn` is an ordinary function, callable normally. The compiler also generates a `Tool` descriptor for it containing:
  - the name
  - a description, from the doc comment
  - a parameter schema, from parameter types and doc comments
  - a result schema
  - its **effect set**
- Parameters must be `Schema`; the result must be `Json`. Both are compile errors otherwise.
- A tool list such as `[weather, read_file]` has type `Tools<uses [network.http, fs.read]>`, whose effect row is the union of its members. A tool list is static, so its effects are known at compile time.
- `@confirm` means each invocation by a model first performs `approval.approve(call)`. The `approval` handler can be:
  - a CLI prompt
  - auto-approve in tests
  - auto-deny in sandboxes

### 5.7 Agents

```nova
type Report {
    summary: String
    sources: List<Url>
}

pub fn research(question: String) -> Result<Report, AiError>
    uses [ai, network.http, fs.read]
{
    let researcher = Agent(
        model: smart,
        instructions: "Research the question. Cite sources.",
        tools: [weather, web_search, read_file],
        budget: Budget(tokens: 50_000, cost: 0.50.usd, time: 2.minutes),
        max_steps: 20,
    )
    researcher.run<Report>(prompt"{question}")
}
```

- `Agent` is a std type. Its constructor is generic over the tool list's effect row, so `run` has effects `ai.generate ∪ <tool effects> ∪ approval (if any tool is @confirm)`.
- **The guarantee:** adding `delete_file` to that tool list is a compile error, because `research` doesn't declare `fs.write`. The error looks like this:

```text
error[E0510]: agent tool `delete_file` uses `fs.write`, which `research` does not declare
  --> src/research.nova:14:44
   |
 2 |     uses [ai, network.http, fs.read]
   |     -------------------------------- declared here
14 |         tools: [weather, web_search, read_file, delete_file],
   |                                                 ^^^^^^^^^^^ uses [fs.write, approval]
   = help: remove the tool, or add `fs.write, approval` to `uses`
```

- **The loop:** call the model with the tools → execute the requested tool calls sequentially → feed back the results → repeat until a final `T` validates, or until `max_steps`/budget is hit.
  - A tool returning `Err` is reported back to the model as a tool error. It does not abort the agent.
- **State is explicit.** `run` returns the result; `run_with(conversation)` takes and returns a `Conversation` value. Persistent memory is the caller's job and uses its own declared effects (e.g. `db`). There is no hidden memory.
- Agents can be tools for other agents (`researcher.as_tool(name:, description:)`). The effects compose through the same rows.
- A runtime sandbox can narrow further. Example: `handle fs.read with fs.read.under("./docs") { research(q) }`.

### 5.8 Streaming

```nova
let story = ai.stream<Story>(prompt: prompt"Write a short story about {topic}")?
for partial in story {
    render(partial?.title, partial?.paragraphs)   // fields are Option while incomplete
}
let final: Story = story.result()?
```

- `Partial<T>` is compiler-generated: every field becomes optional, recursively. Partial updates come from incremental JSON parsing.
- `ai.stream<String>` yields text chunks.
- `.result()` validates the completed value as `generate` does. Retries aren't possible mid-stream, so an invalid final output returns `InvalidOutput`.

### 5.9 Embeddings

```nova
let vecs = ai.embed(chunks, model: embedder)?
let best = vecs.zip(chunks).max_by(fn((v, _)) { v.cosine(query_vec) })
```

- `Vector` is a std type (`F32` elements) with `dot`, `cosine`, `norm`, and `len`.
- Vector stores are libraries (on `fs` or `db`). A simple in-memory `VectorIndex` ships in std.
- Typed dimensions (`Vector<1024>`) are undecided (§11).

### 5.10 Budgets and observability

```nova
handle ai with budget(tokens: 200_000, cost: 2.00.usd, time: 5.minutes) {
    batch_classify(items)
}
```

- `budget(...)` is a delegating handler (§4.5). It counts usage reported by the outer handler and returns `Err(BudgetExceeded)` once a limit is reached.
  - Budgets nest: an inner budget can't spend beyond the outer one.
  - `Agent(budget: …)` is sugar for wrapping `run` this way.
- **Tracing is on by default in the root `ai` handler.** Each operation emits a structured event containing:
  - the call site (file:line, function)
  - the model
  - the prompt template and slots (redacted by default; `--trace-prompts` to keep them)
  - the schema name
  - token counts, cost estimate, latency, and retries
  - validation errors
  - tool calls with their arguments and results
  - the parent span, so agent steps nest under `run`
- MVP output: JSONL in `.nova/traces/`, plus `nova trace` (list, show, tree view of agent runs, cost summary). OpenTelemetry export is post-MVP.
- `cache(...)` is a delegating handler. It memoizes requests by `(model, normalized request)` and is meant for development.

### 5.11 Testing: mocks, record/replay

```nova
test "classifies positive text" {
    handle ai with ai.mock().returns(Sentiment(label: Positive, confidence: 0.9, reason: "…")) {
        assert_eq(analyze("I love it")?.label, Positive)
    }
}
```

- `ai.mock()` serves queued responses (`.returns<T>(v)`, `.returns_text(s)`, `.calls_tool(…)`) or matcher functions (`.on(f)`). Use it for unit tests (`ai.md` §13.1).
- **Record/replay:** `nova test --ai=record` runs against real models and writes responses to `tests/fixtures/ai/<test-name>/<request-hash>.json`. `--ai=replay` serves them back deterministically.
  - The request hash covers the model, the normalized prompt and slots, the schema, the tools, and the parameters.
  - A missing fixture in replay mode fails the test with the request diff.
  - **Replay is the default in CI** (`CI=true`). Local default is `replay`, falling back to `record` when `--ai=auto` is passed.
- Fixtures are reviewed like code; they are small JSON files.

### 5.12 Evals

```nova
type Case { text: String, expected: Label }

eval sentiment_accuracy(case: Case) -> Score
    dataset "evals/sentiment.jsonl"
    threshold 0.90
{
    let got = analyze(case.text)?
    Score.exact(got.label == case.expected)
}

eval summary_quality(case: DocCase) -> Score
    dataset "evals/summaries.jsonl"
    threshold 0.75
{
    let s = summarize(case.doc)?
    Score.judge(model: smart, rubric: "Faithful, concise, covers key points.", output: s, reference: case.reference)
}
```

- `eval` is an item like `test`. The dataset rows are decoded into the parameter type (it must be `Json`). `Err` from the body scores 0.
- `nova eval [name] [--model fast] [--sample 50]` runs live against real models and reports:
  - mean score and pass/fail against the threshold
  - the worst cases
  - tokens and cost
  - delta vs. the stored baseline (`.nova/evals/baseline.json`, updated with `--save-baseline`)
- `Score.exact`, `Score.contains`, `Score.similarity` (embeddings), and `Score.judge` (LLM-as-judge, which itself uses `ai`).
- `nova eval --compare fast,smart` runs the same eval across models side by side.

---

## 6. Architecture

### 6.1 Pipeline

```text
source ─► lexer ─► parser ─► CST (lossless) ─► AST
       ─► name resolution + lowering ─► HIR
       ─► type inference (+ auto traits: Schema, Json, Partial<T>)
       ─► effect inference (+ tool/agent effect rows) ─► typed HIR
       ─► interpreter (MVP)  |  bytecode VM (P7)  |  Cranelift/LLVM (P9+)
```

### 6.2 Repository layout

```text
nova/
  Cargo.toml                 # workspace
  crates/
    nova_syntax/             # lexer (logos), parser, rowan CST, typed AST
    nova_diag/               # diagnostics, rendering (ariadne)
    nova_hir/                # name resolution, modules, lowering
    nova_types/              # types, traits, auto traits, effect checker
    nova_schema/             # Nova type → JSON Schema, validation, Partial<T>, incremental JSON
    nova_interp/             # tree-walking evaluator, values, handler stack
    nova_runtime/            # native root handlers (console, fs, http, time, …)
    nova_ai/                 # model providers, ai root handler, streaming (SSE), tracing, record/replay
    nova_pkg/                # manifest, lockfile, dependency resolution
    nova_cli/                # `nova` binary: run, test, eval, trace, check, update, …
  std/                       # standard library written in Nova (ai.nova, agent.nova, …)
  tests/
    ui/                      # golden tests: .nova + expected diagnostics
    run/                     # programs with expected stdout
    fixtures/ai/             # recorded AI responses
  evals/                     # eval datasets for std/demos
  examples/                  # MVP demos
  docs/
    plan.md  build.md  spec/  decisions/  phases/
```

### 6.3 Key choices

- **Lexer:** `logos`. **CST:** `rowan`. **Parser:** hand-written recursive descent with error recovery.
- **Diagnostics:** `ariadne`. Every error has a code and a spec link.
- **Snapshots:** `insta`.
- **HTTP:** `ureq` (blocking), with an SSE reader for streaming.
- **AI providers** are implemented in Rust (`nova_ai`) for the MVP, behind the handler interface. They can migrate to Nova source in `std/` once the language is capable; callers see no difference.
- **No `salsa` in the MVP.** Write passes as pure functions keyed by file ID so it can be added for the LSP.

### 6.4 Effect checker design notes

- Effect sets are a set of paths plus an optional row variable: `{fs.read, network.http | ε}`.
- Normalize the hierarchy for subset checks. Print the most specific form in diagnostics.
- Record **provenance** for every inferred effect: the call that introduced it. Diagnostics show the chain:

```text
error[E0402]: `summarize` uses `process.exec` but declares `uses [ai]`
  --> src/summary.nova:12:5
   |
 3 | pub fn summarize(doc: Document) -> Summary uses [ai] {
   |                                            --------- declared here
12 |     clean(doc.text)
   |     ^^^^^^^^^^^^^^^ `process.exec` enters here
   |
   = note: clean (src/text.nova:40) → run_formatter (src/text.nova:55) → process.exec.run
   = help: add `process.exec` to `uses`, or handle it: `handle process.exec with … { … }`
```

- Tool lists and `Agent` construction are where E0510-style diagnostics (§5.7) come from. Point at the specific tool, not the whole agent.

These diagnostics are the product. They deserve as much care as the type checker.

---

## 7. Phases

Each phase ends with a tagged release, passing tests, and updated spec files. Tasks are sized for one agent session: one spec section, failing tests first, then the implementation. Detailed task lists go in `docs/phases/PN.md`.

### Phase 0 — Foundations
- Cargo workspace, CI (fmt, clippy, test), `CLAUDE.md` (§9).
- Lexer for §3.1, including newline rules and `prompt` literals.
- Parser + CST + AST for:
  - items: `fn`, `type`, `enum`, `trait`, `impl`, `effect`, `handler`, `use`, `test`, `model`, `tool fn`, `eval`
  - expressions, patterns, attributes, and `uses` clauses
  - `handle … with … { }`
- Error recovery. `nova parse <file>`. A basic `nova fmt` is a stretch goal.
- UI test harness with inline `//~ ERROR E0402` annotations.
- `docs/prior-art.md`.

**Done when:** every code block in this document parses, and malformed inputs produce useful errors.

### Phase 1 — Types
- Modules, `use`, `pub`, name resolution → HIR.
- Structs, enums, generics, traits with bounds, `impl`, methods.
- Bidirectional local inference, `Option`/`Result`/`?`, `T?`, exhaustive `match`, `if let`.
- Mutability checking.
- Auto traits `Schema`/`Json`, validation attributes, and the `Partial<T>` type constructor.
- `Eq`/`Ord`/`Hash`/`Debug` derives.

**Done when:** 300+ typing tests pass with stable diagnostics, and `nova schema <Type>` prints the JSON Schema for any type.

### Phase 2 — Effects
- `effect` declarations with hierarchy, plus built-in declarations in `std/` (including `ai`, `approval`).
- Effect inference with provenance; fixpoint over recursive groups.
- The `pub` rule, upper bounds, implicit row polymorphism.
- `handle … with` typing, including delegation.
- `tool fn` descriptors with effect sets; `Tools<row>` typing; `Agent` effect rows; E0510.
- Root check for `main`. Diagnostics per §6.4.

**Done when:** **Demo 1** passes, and the static half of **Demo 5** (E0510) passes as UI tests.

### Phase 3 — Interpreter + typed AI core
- Tree-walking evaluator; handler stack with tail-resumptive dispatch, delegation, and `abort`.
- Root handlers for all non-AI built-in effects.
- `nova run`, `nova test`. Core stdlib in Nova.
- `nova_schema`: JSON encode/decode, validation.
- `nova_ai`: `openai_compatible` and `anthropic` providers, `model` items and config overrides, the root `ai` handler, `ai.generate<T>` with validation and retry, `ai.mock`.
- Tracing to JSONL (on by default).

**Done when:** **Demo 2** and **Demo 4** (non-streaming part) pass; `tests/run/` has 50+ programs.

### Phase 4 — Tools and agents
- Runtime tool descriptors → provider tool-call formats.
- The `Agent` loop with step limits, tool error feedback, `Conversation`, and `as_tool`.
- `approval` effect and `@confirm`; CLI approval handler.
- `budget(...)` and `cache(...)` delegating handlers.
- `nova trace` (list/show/tree/cost).

**Done when:** **Demo 5** passes end to end.

### Phase 5 — AI reliability, streaming, embeddings
- Record/replay (`--ai=record|replay|auto`), with CI defaulting to replay.
- `eval` items, `nova eval`, scorers, baselines, `--compare`.
- `ai.stream<T>`, incremental JSON parsing, `Partial<T>` values.
- `ai.embed`, `Vector`, in-memory `VectorIndex`.

**Done when:** **Demo 4** (streaming) and **Demo 6** pass.

### Phase 6 — Sandboxing and packages → **v0.1 MVP**
- `nova check --allow`, `nova run --allow`, `--models`; narrowing handlers (`fs.read.under`, `ai.only(model)`).
- `nova.toml`, `nova.lock`, path/git deps, effect recording, effect diff on update, `nova why-effect`.

**Done when:** **Demo 3** passes, and an effect-gaining dependency update is refused in a test.

### Post-MVP (order tentative)
- **P7 Bytecode VM:** full one-shot `resume`, generators.
- **P8 Concurrency:** `task`/`await` as an effect, `Send`, actors, supervisors. This also brings parallel agent tool calls.
- **P9 Native:** Cranelift, Rust runtime, GC, single static binary.
- **P10 Tooling:** LSP (salsa), full formatter, `nova doc`, OpenTelemetry export.
- **P11 Ecosystem:** user derives / general reflection, HTTP server and SQL libraries, registry with effect metadata, LLVM release backend, WASM target, more providers.

---

## 8. MVP demos

Each lives in `examples/` and runs in CI (AI demos in replay mode).

**Demo 1 — Effect violation caught.**
An "AI-written" `summarize(doc) uses [ai]` whose helper, three calls deep, runs `process.exec`. `nova check` fails with the provenance diagnostic (§6.4).

**Demo 2 — Mocking via handlers.**
`pub fn forecast(city) uses [network.http, time.clock]` runs against a real API under `nova run`. Under `nova test` it runs with `MockHttp` + `FixedClock`, with zero code changes.

**Demo 3 — Sandboxed execution.**
A plugin host loads `plugins/*.nova`.
- `nova check --allow console,fs.read,ai` rejects a plugin that writes files.
- At runtime, `fs.read.under("./data")` permits legitimate reads and returns `SandboxError` for `/etc/passwd`.
- `--models fast` blocks a plugin from reaching the cloud model.

**Demo 4 — Typed AI.**
`analyze(text) -> Result<Sentiment, AiError> uses [ai]` runs against a local OpenAI-compatible server.
- A malformed output triggers a retry with validation feedback, then a typed `InvalidOutput`.
- `ai.stream<Story>` renders partial results live.

**Demo 5 — Effect-bounded agent.**
A research agent with tools `[web_search, read_file]` answers a question into a typed `Report`.
- Adding `delete_file` fails compilation with E0510.
- A deliberately looping prompt is stopped by its budget, returning `BudgetExceeded`.
- `nova trace` shows the step tree, tool calls, tokens, and cost.

**Demo 6 — Reliable AI in CI.**
- `nova test` passes deterministically from recorded fixtures with no network.
- `nova eval sentiment_accuracy --compare fast,smart` reports scores, cost, and the baseline delta.

---

## 9. Working with AI agents

- **The spec is the contract.** Every task cites a `docs/spec/` section. Ambiguity means stop and ask, not invent. Spec changes come with a decision record.
- **Tests first.** Each task begins with failing UI/run tests derived from the spec.
- **Small tasks.** ≤ ~500 changed lines. Phase task lists live in `docs/phases/PN.md` with acceptance criteria.
- **Snapshot discipline.** Diagnostic and AI-fixture snapshot changes get human review, never wholesale acceptance.
- **No new dependencies** without a decision record.
- **No live model calls in CI.** Live calls happen only in `nova eval` and `--ai=record`, run by a human.
- **`CLAUDE.md`** at the repo root covers commands, crate boundaries, the error-code registry, and these rules.
- **Dogfood the thesis.** After Phase 2, every `pub` item in std declares its effects. Agent-written code is checked by the system it builds.

---

## 10. Risks

| Risk | Mitigation |
|---|---|
| MVP scope grew substantially with AI co-equal | Phases 3–5 are sliced so each ends in a working demo. If time slips, cut embeddings, then streaming, then evals (record/replay stays). |
| Provider APIs churn | The compiler knows no vendor. Providers live behind handlers in `nova_ai`/std; OpenAI-compatible covers most local servers. |
| Effect annotations feel noisy | Inference for private code, row polymorphism, hierarchical names. Measure annotation density in std/demos and revisit D5 if it's painful. |
| Effect and agent errors are confusing | Provenance chains and E0510 are phase exit criteria, not polish. |
| Users assume `Prompt` slots stop prompt injection | The docs state they don't. Safety comes from effect-bounded tools, `@confirm`, and sandboxes. |
| Leaking prompts and secrets through traces | Redaction is on by default; prompts are stored only with `--trace-prompts`; secrets never come from source. |
| Tree-walker doesn't extend to full `resume` | Accepted. P7 replaces the evaluator; typed HIR is the stable interface. |
| Scope creep toward `plan.md` | Non-goals (§1) are binding until v0.1. |
| Prior art (Koka, Effekt, Flix, OCaml 5, Unison, Roc; BAML, DSPy, Marvin for typed LLM calls) | `docs/prior-art.md` in Phase 0. Nova's angle: effects + AI in one compiler, effect-bounded agents, effects in lockfiles, mainstream syntax. |

---

## 11. Open questions

To resolve during Phases 0–3. Each becomes a decision record.

1. ~~Effect operation call syntax~~ — **resolved** in `effects.md` §5: unique-suffix resolution, plus merged module/effect names.
2. ~~Handler syntax~~ — **resolved** in `effects.md` §6: `handler E for T`, `handle E with expr`, and inline handlers.
3. **`mut` semantics:** deep vs. shallow; `mut` fields inside immutable types.
4. **Generics strategy** (decide before P9; likely monomorphization).
5. **Numeric tower:** sized ints, `Decimal`/`Money` (needed for `cost: 0.50.usd`).
6. **Lambda short syntax**, trailing closures.
7. ~~Can one handler type implement several effects?~~ — **resolved:** yes, with one `handler` declaration per node (`effects.md` §6.1).
8. ~~Default handlers in `test` blocks~~ — **resolved:** `ai` → replay, `approval` → auto-approve; other built-ins are real (`effects.md` §8.4).
9. ~~Lockfile effect granularity~~ — **resolved for v0.1:** a union per package; per-item detail via `nova why-effect` (`effects.md` §10.3).
10. **Prompt roles:** is `system:` a named argument only, or should prompt literals support role sections?
11. **`agent` item syntax:** promote `Agent(...)` to a declaration if the std API proves clunky in Demo 5.
12. **Typed vector dimensions** (`Vector<1024>`): needs const generics.
13. **Cost accounting:** where price tables live and how stale prices are handled.
14. **Name:** "Nova" collides with existing projects; check before any public release.
