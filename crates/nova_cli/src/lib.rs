//! The `nova` command-line driver.

use std::path::{Path, PathBuf};

use nova_diag::{Diagnostic, SourceMap, render};
use nova_interp::Interp;
use nova_types::{Program, RUNTIME_ROOT_LEAVES, compile};

const USAGE: &str = "\
Usage: nova <file.nova>        Check and run `main` (same as `nova run`)
       nova <command> [args]

Commands:
  run <file>               Check and run `main`
  test [path] [filter]     Run `test` blocks in a file or directory (default: .)
  check <path>             Report diagnostics without running
  --version                Print the version
";

/// Exit codes: 0 success, 1 diagnostics or test failures, 2 usage/IO errors, 101 panic.
pub fn run_cli(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("run") => match args.get(1) {
            Some(file) => cmd_run(Path::new(file)),
            None => usage_error("`nova run` needs a file"),
        },
        Some("test") => {
            let path = args.get(1).map_or(".", String::as_str);
            cmd_test(Path::new(path), args.get(2).map(String::as_str))
        }
        Some("check") => match args.get(1) {
            Some(path) => cmd_check(Path::new(path)),
            None => usage_error("`nova check` needs a path"),
        },
        Some("--version" | "-V") => {
            println!("nova {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--help" | "-h" | "help") | None => {
            print!("{USAGE}");
            0
        }
        Some(file) if file.ends_with(".nova") => cmd_run(Path::new(file)),
        Some(other) => usage_error(&format!("unknown command `{other}`")),
    }
}

fn usage_error(msg: &str) -> i32 {
    eprintln!("error: {msg}\n\n{USAGE}");
    2
}

/// Diagnostics for one file, as plain data (used by the UI test harness).
pub fn check_source(name: &str, text: &str) -> (SourceMap, Vec<Diagnostic>) {
    let mut sources = SourceMap::default();
    let (_, diags) = compile(&mut sources, name, text);
    (sources, diags)
}

/// Loads and checks a file, printing diagnostics to stderr.
fn load(path: &Path) -> Result<(SourceMap, Program), i32> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return Err(2);
        }
    };
    let mut sources = SourceMap::default();
    let (program, diags) = compile(&mut sources, &path.display().to_string(), &text);
    print_diags(&diags, &sources);
    program.map(|p| (sources, p)).ok_or(1)
}

fn print_diags(diags: &[Diagnostic], sources: &SourceMap) {
    for d in diags {
        eprintln!("{}", render(d, sources));
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    if errors > 0 {
        eprintln!("error: aborting due to {errors} previous error{}", if errors == 1 { "" } else { "s" });
    }
}

fn cmd_run(path: &Path) -> i32 {
    let Ok((_, program)) = load(path) else { return 1 };
    if program.fn_decl("main").is_none() {
        eprintln!("error[E0600]: no `main` function in {}", path.display());
        return 1;
    }
    let roots = main_roots(&program);
    match Interp::new(&program, &roots).run_main() {
        Ok(_) => 0,
        Err(msg) => {
            eprintln!("panic: {msg}");
            101
        }
    }
}

/// Root handlers for `main`: exactly the built-in leaves it declares (effects §8.4).
fn main_roots(program: &Program) -> Vec<&'static str> {
    let main = program.fn_decl("main").expect("main exists");
    let declared: Vec<String> = main
        .uses
        .iter()
        .flat_map(|c| &c.paths)
        .filter_map(|p| program.effects.lookup(&p.dotted()))
        .flat_map(|n| program.effects.leaves(n))
        .collect();
    RUNTIME_ROOT_LEAVES.iter().copied().filter(|l| declared.iter().any(|d| d == l)).collect()
}

fn cmd_check(path: &Path) -> i32 {
    let files = match nova_files(path) {
        Ok(f) => f,
        Err(code) => return code,
    };
    let mut failed = false;
    for file in &files {
        failed |= load(file).is_err();
    }
    if failed {
        1
    } else {
        eprintln!("checked {} file{}: ok", files.len(), if files.len() == 1 { "" } else { "s" });
        0
    }
}

fn cmd_test(path: &Path, filter: Option<&str>) -> i32 {
    let files = match nova_files(path) {
        Ok(f) => f,
        Err(code) => return code,
    };
    let (mut passed, mut failed, mut broken) = (0, 0, 0);
    let mut failures = Vec::new();
    for file in &files {
        let Ok((_, program)) = load(file) else {
            broken += 1;
            continue;
        };
        let tests: Vec<_> = program.tests().filter(|t| filter.is_none_or(|f| t.name.contains(f))).collect();
        if tests.is_empty() {
            continue;
        }
        println!("running {} test{} from {}", tests.len(), if tests.len() == 1 { "" } else { "s" }, file.display());
        for test in tests {
            // Fresh root handlers per test (effects §8.4).
            match Interp::new(&program, RUNTIME_ROOT_LEAVES).run_test(test) {
                Ok(()) => {
                    println!("test {} ... ok", test.name);
                    passed += 1;
                }
                Err(msg) => {
                    println!("test {} ... FAILED", test.name);
                    failures.push((file.display().to_string(), test.name.clone(), msg));
                    failed += 1;
                }
            }
        }
        println!();
    }
    if !failures.is_empty() {
        println!("failures:\n");
        for (file, name, msg) in &failures {
            println!("---- {name} ({file}) ----\npanic: {msg}\n");
        }
    }
    let status = if failed == 0 && broken == 0 { "ok" } else { "FAILED" };
    let broken_note = if broken > 0 { format!("; {broken} file(s) failed to compile") } else { String::new() };
    println!("test result: {status}. {passed} passed; {failed} failed{broken_note}");
    if failed == 0 && broken == 0 { 0 } else { 1 }
}

/// A single `.nova` file, or all `.nova` files under a directory (sorted).
fn nova_files(path: &Path) -> Result<Vec<PathBuf>, i32> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        eprintln!("error: {} does not exist", path.display());
        return Err(2);
    }
    let mut out = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if p.is_dir() {
                if !name.starts_with('.') && name != "target" {
                    stack.push(p);
                }
            } else if name.ends_with(".nova") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}
