//! End-to-end tests of the `nova` binary.

use std::path::PathBuf;
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn nova(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nova")).args(args).current_dir(root()).output().expect("run nova")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn hello_world_runs() {
    let o = nova(&["run", "examples/hello_world.nova"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stdout(&o), "Hello, world!\n");
}

#[test]
fn bare_file_argument_runs_it() {
    let o = nova(&["examples/hello_world.nova"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stdout(&o), "Hello, world!\n");
}

#[test]
fn hello_world_tests_pass() {
    let o = nova(&["test", "examples/hello_world.nova"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("test hello world ... ok"), "{out}");
    assert!(out.contains("test main prints hello world ... ok"), "{out}");
    assert!(out.contains("test result: ok. 2 passed; 0 failed"), "{out}");
}

#[test]
fn test_filter_selects_by_name() {
    let o = nova(&["test", "examples/hello_world.nova", "prints"]);
    assert!(stdout(&o).contains("test result: ok. 1 passed; 0 failed"));
}

#[test]
fn failing_test_exits_nonzero_with_message() {
    let o = nova(&["test", "tests/cli/failing_test.nova"]);
    assert_eq!(o.status.code(), Some(1));
    let out = stdout(&o);
    assert!(out.contains("test arithmetic ... FAILED"), "{out}");
    assert!(out.contains("left: 2\n right: 3"), "{out}");
}

#[test]
fn effect_violation_blocks_run() {
    let o = nova(&["run", "tests/ui/effects/undeclared_via_helper.nova"]);
    assert_eq!(o.status.code(), Some(1));
    let err = stderr(&o);
    assert!(err.contains("error[E0402]: `main` uses `console` but has no `uses` clause"), "{err}");
    assert!(err.contains("helper (tests/ui/effects/undeclared_via_helper.nova:1) → console.print"), "{err}");
    assert!(stdout(&o).is_empty(), "nothing may run when checking fails");
}

#[test]
fn panic_exits_101() {
    let o = nova(&["run", "tests/cli/panics.nova"]);
    assert_eq!(o.status.code(), Some(101));
    assert!(stderr(&o).contains("panic: boom"));
}

#[test]
fn version_and_unknown_command() {
    assert!(stdout(&nova(&["--version"])).starts_with("nova "));
    assert_eq!(nova(&["frobnicate"]).status.code(), Some(2));
}
