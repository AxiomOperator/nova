# Nova

A statically typed language whose functions declare what they're allowed to do —
and whose compiler enforces it — with first-class AI.

```nova
fn greeting(name: String) -> String {
    "Hello, {name}!"
}

pub fn main() uses [console] {
    console.print(greeting("world"))
}

test "hello world" {
    assert_eq(greeting("world"), "Hello, world!")
}
```

## Try it

```bash
cargo install --path crates/nova_cli             # puts `nova` in ~/.cargo/bin

nova examples/hello_world.nova                   # Hello, world!
nova test examples/hello_world.nova              # runs the test blocks
nova check tests/ui/effects/undeclared_via_helper.nova   # see an effect error
```

Without installing, use `cargo run -q -- <args>` in place of `nova`.

Status: early vertical slice (see `docs/decisions/0001-vertical-slice.md`).
Plan: `docs/build.md`. Specs: `docs/spec/`.
