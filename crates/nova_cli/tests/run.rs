//! Run tests: each `tests/run/*.nova` program is executed with `nova run`, and
//! its stdout must equal the sibling `.stdout` file.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn run_tests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("tests/run"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "nova"))
        .collect();
    files.sort();
    assert!(!files.is_empty());

    let mut failures = Vec::new();
    for file in &files {
        let expected = std::fs::read_to_string(file.with_extension("stdout")).unwrap_or_default();
        let out = Command::new(env!("CARGO_BIN_EXE_nova")).arg("run").arg(file).output().unwrap();
        let actual = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || actual != expected {
            failures.push(format!(
                "{}: exit {:?}\n--- expected\n{expected}--- actual\n{actual}--- stderr\n{}",
                file.display(),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
