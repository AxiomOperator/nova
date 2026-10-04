//! Semantic analysis: the effect hierarchy, name resolution, and effect checking.

mod check;
pub mod effects;

pub use check::{Builtin, Callee, Program, RUNTIME_ROOT_LEAVES, check};

use nova_diag::{Diagnostic, SourceMap};

/// Source of the built-in effect declarations (`std/effects.nova`).
pub const STD_EFFECTS: &str = include_str!("../../../std/effects.nova");

/// Parses and checks one user file together with the standard library.
/// Returns a program only when there are no errors (warnings are allowed).
pub fn compile(sources: &mut SourceMap, name: &str, text: &str) -> (Option<Program>, Vec<Diagnostic>) {
    let std_id = sources.add("<std>/effects.nova", STD_EFFECTS);
    let user_id = sources.add(name, text);
    let (std_file, mut diags) = nova_syntax::parse_file(std_id, STD_EFFECTS);
    let (user_file, user_diags) = nova_syntax::parse_file(user_id, text);
    diags.extend(user_diags);
    if diags.iter().any(Diagnostic::is_error) {
        return (None, diags);
    }
    let (program, check_diags) = check(vec![std_file, user_file], 1, sources);
    diags.extend(check_diags);
    let ok = !diags.iter().any(Diagnostic::is_error);
    (ok.then_some(program), diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(src: &str) -> Vec<&'static str> {
        let mut sources = SourceMap::default();
        compile(&mut sources, "t.nova", src).1.iter().map(|d| d.code).collect()
    }

    #[test]
    fn hello_world_checks() {
        assert!(codes("pub fn main() uses [console] {\n    console.print(\"hi\")\n}\n").is_empty());
    }

    #[test]
    fn undeclared_effect_through_helper() {
        let src = "fn helper() {\n    console.print(\"hi\")\n}\n\npub fn main() {\n    helper()\n}\n";
        let mut sources = SourceMap::default();
        let (_, diags) = compile(&mut sources, "t.nova", src);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "E0402");
        assert_eq!(diags[0].notes[0], "helper (t.nova:1) → console.print");
    }

    #[test]
    fn mutual_recursion_reaches_fixpoint() {
        let src = "fn a(n: Int) {\n    b(n)\n}\n\nfn b(n: Int) {\n    if n > 0 {\n        a(n - 1)\n    } else {\n        console.print(\"done\")\n    }\n}\n\npub fn main() uses [console] {\n    a(3)\n}\n";
        assert!(codes(src).is_empty());
    }

    #[test]
    fn handler_discharges_effect() {
        let src = "pub fn quiet() {\n    handle console with {\n        fn print(t) { }\n        fn eprint(t) { }\n        fn read_line() { None }\n    } {\n        console.print(\"x\")\n    }\n}\n";
        assert!(codes(src).is_empty());
    }

    #[test]
    fn handler_errors() {
        assert_eq!(
            codes(
                "fn f() {\n    handle console with {\n        fn print(t) { }\n    } {\n        console.print(\"x\")\n    }\n}\n"
            ),
            vec!["E0406"]
        );
        assert_eq!(codes("fn f() {\n    abort 1\n}\n"), vec!["E0409"]);
    }

    #[test]
    fn user_effect_in_main_needs_handler() {
        let src =
            "effect kv {\n    fn get(k: String) -> String?\n}\n\npub fn main() uses [kv] {\n    kv.get(\"a\")\n}\n";
        assert_eq!(codes(src), vec!["E0408"]);
    }

    #[test]
    fn unused_declared_effect_warns() {
        assert_eq!(codes("pub fn f() uses [console] {\n}\n"), vec!["W0401"]);
    }

    #[test]
    fn immutable_assignment() {
        assert_eq!(codes("fn f() {\n    let x = 1\n    x = 2\n}\n"), vec!["E0104"]);
    }
}
