//! UI tests: every `tests/ui/**/*.nova` file is checked, and its diagnostics
//! must match inline annotations exactly (P0-04 format, `check` mode only):
//!
//!     foo()   //~ ERROR E0402
//!     //~^ WARN W0401          (refers to the line above; `^^` = two lines up)
//!
//! A file with no annotations must produce no diagnostics.

use std::path::{Path, PathBuf};

use nova_diag::{Severity, render};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|e| e == "nova") {
            out.push(p);
        }
    }
}

/// `(line, "ERROR"|"WARN", code)` expected by annotations.
fn annotations(text: &str) -> Vec<(usize, String, String)> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some(pos) = line.find("//~") else { continue };
        let rest = &line[pos + 3..];
        let carets = rest.chars().take_while(|&c| c == '^').count();
        let mut words = rest[carets..].split_whitespace();
        let (Some(level), Some(code)) = (words.next(), words.next()) else {
            panic!("malformed annotation on line {}: {line}", i + 1);
        };
        out.push((i + 1 - carets, level.to_string(), code.to_string()));
    }
    out.sort();
    out
}

#[test]
fn ui_tests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    collect(&root.join("tests/ui"), &mut files);
    files.sort();
    assert!(!files.is_empty());

    let mut failures = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&root).unwrap().display().to_string();
        let text = std::fs::read_to_string(file).unwrap();
        let (sources, diags) = nova_cli::check_source(&rel, &text);
        let mut actual: Vec<(usize, String, String)> = diags
            .iter()
            .map(|d| {
                let span = d.primary_span().expect("diagnostic has a span");
                let line = sources.get(span.file).line_col(span.start).0;
                let level = if d.severity == Severity::Error { "ERROR" } else { "WARN" };
                (line, level.to_string(), d.code.to_string())
            })
            .collect();
        actual.sort();
        let expected = annotations(&text);
        if actual != expected {
            let rendered: String = diags.iter().map(|d| render(d, &sources)).collect();
            failures.push(format!("{rel}\n  expected: {expected:?}\n  actual:   {actual:?}\n{rendered}"));
        }
    }
    assert!(failures.is_empty(), "{} UI test(s) failed:\n\n{}", failures.len(), failures.join("\n"));
}
