//! Name resolution and effect checking (docs/spec/effects.md §5–§7).
//!
//! Types are not checked yet (Phase 1); the interpreter reports dynamic type
//! errors as panics until then.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nova_diag::{Diagnostic, SourceMap, Span};
use nova_syntax::ast::*;

use crate::effects::{EffectTree, OpLookup};

/// Built-in leaves the runtime can provide root handlers for in this build.
pub const RUNTIME_ROOT_LEAVES: &[&str] = &["console"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callee {
    Fn(String),
    Builtin(Builtin),
    Op { leaf: String, op: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Assert,
    AssertEq,
    Panic,
    Some,
}

impl Builtin {
    pub fn lookup(name: &str) -> Option<Builtin> {
        Some(match name {
            "assert" => Builtin::Assert,
            "assert_eq" => Builtin::AssertEq,
            "panic" => Builtin::Panic,
            "Some" => Builtin::Some,
            _ => return None,
        })
    }

    /// Accepted positional argument counts (inclusive).
    fn arity(self) -> (usize, usize) {
        match self {
            Builtin::Assert => (1, 2),
            Builtin::AssertEq => (2, 3),
            Builtin::Panic | Builtin::Some => (1, 1),
        }
    }
}

/// Values in scope everywhere besides locals.
const BUILTIN_VALUES: &[&str] = &["None"];

/// A checked program, ready for the interpreter.
pub struct Program {
    pub files: Vec<File>,
    pub effects: EffectTree,
    pub callees: HashMap<ExprId, Callee>,
    fns: HashMap<String, (usize, usize)>,
    tests: Vec<(usize, usize)>,
}

impl Program {
    pub fn fn_decl(&self, name: &str) -> Option<&FnDecl> {
        let &(f, i) = self.fns.get(name)?;
        match &self.files[f].items[i] {
            Item::Fn(decl) => Some(decl),
            _ => None,
        }
    }

    pub fn tests(&self) -> impl Iterator<Item = &TestDecl> {
        self.tests.iter().filter_map(|&(f, i)| match &self.files[f].items[i] {
            Item::Test(t) => Some(t),
            _ => None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Origin {
    span: Span,
    via: Via,
}

#[derive(Clone, Debug, PartialEq)]
enum Via {
    /// Performed directly, e.g. `console.print`.
    Op(String),
    /// Arrived through a call to this function.
    Fn(String),
}

/// Inferred effects of a body: leaf path → first place it entered.
type Row = BTreeMap<String, Origin>;

fn add(row: &mut Row, leaf: String, origin: Origin) {
    row.entry(leaf).or_insert(origin);
}

/// Checks parsed files. The first `std_files` files are the standard library.
pub fn check(files: Vec<File>, std_files: usize, sources: &SourceMap) -> (Program, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut effects = EffectTree::default();
    let mut fns: HashMap<String, (usize, usize)> = HashMap::new();
    let mut tests = Vec::new();
    let mut test_names: HashMap<&str, Span> = HashMap::new();

    for (fi, file) in files.iter().enumerate() {
        for (ii, item) in file.items.iter().enumerate() {
            match item {
                Item::Effect(decl) => effects.add_decl(decl, None, fi < std_files, &mut diags),
                Item::Fn(decl) => {
                    if let Some(&(pf, pi)) = fns.get(&decl.name.name) {
                        let Item::Fn(prev) = &files[pf].items[pi] else { unreachable!() };
                        diags.push(
                            Diagnostic::error(
                                "E0106",
                                format!("function `{}` is defined more than once", decl.name.name),
                            )
                            .primary(decl.name.span, "duplicate definition")
                            .secondary(prev.name.span, "first defined here"),
                        );
                    } else {
                        fns.insert(decl.name.name.clone(), (fi, ii));
                    }
                }
                Item::Test(test) => {
                    if let Some(&prev) = test_names.get(test.name.as_str()) {
                        diags.push(
                            Diagnostic::error("E0108", format!("test `{}` is defined more than once", test.name))
                                .primary(test.name_span, "duplicate test name")
                                .secondary(prev, "first defined here"),
                        );
                    } else {
                        test_names.insert(&test.name, test.name_span);
                    }
                    tests.push((fi, ii));
                }
            }
        }
    }

    let mut checker = Checker {
        files: &files,
        effects: &effects,
        sources,
        fns: &fns,
        declared: HashMap::new(),
        inferred: HashMap::new(),
        emit: false,
        diags: Vec::new(),
        callees: HashMap::new(),
        scopes: Vec::new(),
        inline_op_depth: 0,
    };
    checker.run(&tests);
    diags.append(&mut checker.diags);
    let callees = std::mem::take(&mut checker.callees);
    drop(checker);

    diags.sort_by_key(|d| d.primary_span().map(|s| (s.file, s.start)));
    (Program { files, effects, callees, fns, tests }, diags)
}

struct Checker<'a> {
    files: &'a [File],
    effects: &'a EffectTree,
    sources: &'a SourceMap,
    fns: &'a HashMap<String, (usize, usize)>,
    /// `None`: row is inferred. `Some`: declared leaf set (pub/`main` without a clause is pure).
    declared: HashMap<String, Option<BTreeSet<String>>>,
    inferred: HashMap<String, Row>,
    /// Diagnostics are only reported on the final pass.
    emit: bool,
    diags: Vec<Diagnostic>,
    callees: HashMap<ExprId, Callee>,
    /// Local bindings: name → mutable.
    scopes: Vec<HashMap<String, bool>>,
    inline_op_depth: usize,
}

impl<'a> Checker<'a> {
    fn report(&mut self, d: Diagnostic) {
        if self.emit {
            self.diags.push(d);
        }
    }

    fn fn_decl(&self, name: &str) -> &'a FnDecl {
        let (f, i) = self.fns[name];
        match &self.files[f].items[i] {
            Item::Fn(decl) => decl,
            _ => unreachable!(),
        }
    }

    fn run(&mut self, tests: &[(usize, usize)]) {
        let mut names: Vec<&String> = self.fns.keys().collect();
        names.sort();

        for name in &names {
            let decl = self.fn_decl(name);
            let declared = self.declared_set(decl);
            self.declared.insert((*name).clone(), declared);
        }

        // Fixpoint over inferred rows (§7.2 rule 4). Rows only grow, so this terminates.
        for _ in 0..1000 {
            let mut changed = false;
            for name in &names {
                let row = self.walk_fn(self.fn_decl(name));
                let old = self.inferred.get(*name);
                if old.is_none_or(|o| !o.keys().eq(row.keys())) {
                    changed = true;
                }
                self.inferred.insert((*name).clone(), row);
            }
            if !changed {
                break;
            }
        }

        // Final pass: report resolution errors and record callees.
        self.emit = true;
        self.declared.clear();
        for name in &names {
            let decl = self.fn_decl(name);
            let declared = self.declared_set(decl);
            self.declared.insert((*name).clone(), declared);
        }
        for name in &names {
            let decl = self.fn_decl(name);
            let errors_before = self.diags.iter().filter(|d| d.is_error()).count();
            let row = self.walk_fn(decl);
            let body_has_errors = self.diags.iter().filter(|d| d.is_error()).count() > errors_before;
            self.check_declared(decl, &row, body_has_errors);
            self.inferred.insert((*name).clone(), row);
        }
        if self.fns.contains_key("main") {
            self.check_main(self.fn_decl("main"));
        }
        for &(f, i) in tests {
            let Item::Test(test) = &self.files[f].items[i] else { continue };
            self.scopes = vec![HashMap::new()];
            let mut row = Row::new();
            self.walk_block(&test.body, &mut row);
            self.check_root_row(&row, &format!("test `{}`", test.name), "the test");
        }
    }

    /// Resolves a `uses` clause to leaves; `pub` functions and `main` without one are pure.
    fn declared_set(&mut self, decl: &FnDecl) -> Option<BTreeSet<String>> {
        match &decl.uses {
            Some(clause) => {
                let mut set = BTreeSet::new();
                for path in &clause.paths {
                    match self.effects.lookup(&path.dotted()) {
                        Some(node) => set.extend(self.effects.leaves(node)),
                        None => self.report(self.unknown_effect(path)),
                    }
                }
                Some(set)
            }
            None if decl.is_pub || decl.name.name == "main" => Some(BTreeSet::new()),
            None => None,
        }
    }

    fn unknown_effect(&self, path: &EffectPath) -> Diagnostic {
        Diagnostic::error("E0401", format!("unknown effect `{}`", path.dotted())).primary(path.span, "not declared")
    }

    /// The row callers see for `name`.
    fn visible_row(&self, name: &str) -> BTreeSet<String> {
        match self.declared.get(name) {
            Some(Some(set)) => set.clone(),
            _ => self.inferred.get(name).map(|r| r.keys().cloned().collect()).unwrap_or_default(),
        }
    }

    fn walk_fn(&mut self, decl: &FnDecl) -> Row {
        let params = decl.params.iter().map(|p| (p.name.name.clone(), false)).collect();
        self.scopes = vec![params];
        let mut row = Row::new();
        self.walk_block(&decl.body, &mut row);
        row
    }

    // ---- post-inference checks ----

    fn check_declared(&mut self, decl: &FnDecl, row: &Row, body_has_errors: bool) {
        let Some(Some(declared)) = self.declared.get(&decl.name.name).cloned() else { return };
        let name = &decl.name.name;
        let excess: BTreeSet<String> = row.keys().filter(|l| !declared.contains(*l)).cloned().collect();
        if let Some(first) = excess.iter().next() {
            let origin = &row[first];
            let excess_s = self.effects.display(&excess);
            let all: BTreeSet<String> = row.keys().chain(declared.iter()).cloned().collect();
            let mut d = match &decl.uses {
                Some(clause) => Diagnostic::error(
                    "E0402",
                    format!("`{name}` uses `{excess_s}` but declares `uses [{}]`", self.effects.display(&declared)),
                )
                .secondary(clause.span, "declared here")
                .help(format!("add `{excess_s}` to `uses`, or handle it: `handle {first} with {{ ... }} {{ ... }}`")),
                None => Diagnostic::error("E0402", format!("`{name}` uses `{excess_s}` but has no `uses` clause"))
                    .secondary(decl.name.span, "public functions are pure unless they declare effects")
                    .help(format!("add `uses [{}]` to the signature", self.effects.display(&all))),
            };
            d = d.primary(origin.span, format!("`{first}` enters here")).note(self.chain(first, origin));
            self.report(d);
        }
        // A body with errors may be missing effects, so "unused" would be noise.
        if let (Some(clause), false) = (&decl.uses, body_has_errors) {
            let unused: BTreeSet<String> = declared.iter().filter(|l| !row.contains_key(*l)).cloned().collect();
            if !unused.is_empty() {
                let unused_s = self.effects.display(&unused);
                self.report(
                    Diagnostic::warning("W0401", format!("`{name}` declares `{unused_s}` but never uses it"))
                        .primary(clause.span, "unused effect")
                        .help("remove it from `uses` unless it is reserved for API stability"),
                );
            }
        }
    }

    fn check_main(&mut self, main: &FnDecl) {
        if let Some(param) = main.params.first() {
            self.report(
                Diagnostic::error("E0601", "`main` must not take parameters")
                    .primary(param.name.span, "read arguments with `process.env.args()` instead"),
            );
        }
        let Some(Some(declared)) = self.declared.get("main").cloned() else { return };
        for leaf in &declared {
            if RUNTIME_ROOT_LEAVES.contains(&leaf.as_str()) {
                continue;
            }
            let span = main.uses.as_ref().map_or(main.name.span, |c| c.span);
            let builtin = self.effects.lookup(leaf).is_some_and(|n| self.effects.nodes[n].builtin);
            let help = if builtin {
                "this build has no root handler for it yet".to_string()
            } else {
                format!("remove it from `uses` and handle it inside `main`: `handle {leaf} with {{ ... }} {{ ... }}`")
            };
            self.report(
                Diagnostic::error("E0408", format!("`{leaf}` cannot be provided by the runtime"))
                    .primary(span, "declared by `main`")
                    .help(help),
            );
        }
    }

    fn check_root_row(&mut self, row: &Row, what: &str, inside: &str) {
        for (leaf, origin) in row {
            if RUNTIME_ROOT_LEAVES.contains(&leaf.as_str()) {
                continue;
            }
            self.report(
                Diagnostic::error("E0408", format!("{what} performs `{leaf}`, which has no root handler"))
                    .primary(origin.span, format!("`{leaf}` enters here"))
                    .note(self.chain(leaf, origin))
                    .help(format!("handle it inside {inside}: `handle {leaf} with {{ ... }} {{ ... }}`")),
            );
        }
    }

    /// Provenance chain for a leaf (§12.1), e.g. `helper (x.nova:3) → console.print`.
    fn chain(&self, leaf: &str, origin: &Origin) -> String {
        let mut parts = Vec::new();
        let mut cur = origin.clone();
        for _ in 0..8 {
            match &cur.via {
                Via::Op(path) => {
                    parts.push(path.clone());
                    return parts.join(" → ");
                }
                Via::Fn(callee) => {
                    let loc = self.sources.location(self.fn_decl(callee).name.span);
                    parts.push(format!("{callee} ({loc})"));
                    match self.inferred.get(callee).and_then(|r| r.get(leaf)) {
                        Some(next) => cur = next.clone(),
                        None => return parts.join(" → "),
                    }
                }
            }
        }
        parts.push("…".into());
        parts.join(" → ")
    }

    // ---- scopes ----

    fn local(&self, name: &str) -> Option<bool> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn define(&mut self, name: &str, mutable: bool) {
        self.scopes.last_mut().expect("scope").insert(name.to_string(), mutable);
    }

    // ---- walking ----

    fn walk_block(&mut self, block: &Block, row: &mut Row) {
        self.scopes.push(HashMap::new());
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let { mutable, name, value, .. } => {
                    self.walk_expr(value, row);
                    self.define(&name.name, *mutable);
                }
                Stmt::Assign { target, value, .. } => {
                    match self.local(&target.name) {
                        None => self.report(
                            Diagnostic::error("E0101", format!("cannot find variable `{}` in this scope", target.name))
                                .primary(target.span, "not found"),
                        ),
                        Some(false) => self.report(
                            Diagnostic::error("E0104", format!("cannot assign to immutable binding `{}`", target.name))
                                .primary(target.span, "bound with `let`")
                                .help("declare it with `var` to allow reassignment"),
                        ),
                        Some(true) => {}
                    }
                    self.walk_expr(value, row);
                }
                Stmt::Expr(e) => self.walk_expr(e, row),
            }
        }
        self.scopes.pop();
    }

    fn walk_expr(&mut self, e: &Expr, row: &mut Row) {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Error => {}
            ExprKind::Str(segs) => {
                for seg in segs {
                    if let StrSeg::Expr(inner) = seg {
                        self.walk_expr(inner, row);
                    }
                }
            }
            ExprKind::Name(id) => self.resolve_value(id),
            ExprKind::Field(base, _) => {
                let path = flatten_path(e);
                let first = path.as_ref().map(|p| p[0].name.as_str());
                match first {
                    Some(name) if self.local(name).is_none() && self.effects.root(name).is_some() => {
                        let dotted = path.unwrap().iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
                        self.report(
                            Diagnostic::error("E0417", format!("operation `{dotted}` used as a value"))
                                .primary(e.span, "operations can only be called"),
                        );
                    }
                    _ => {
                        self.walk_expr(base, row);
                        self.report(
                            Diagnostic::error("E0107", "field access is not supported yet")
                                .primary(e.span, "planned for Phase 1 (types)"),
                        );
                    }
                }
            }
            ExprKind::Call { callee, args } => {
                for arg in args {
                    self.walk_expr(&arg.value, row);
                }
                self.walk_call(e, callee, args, row);
            }
            ExprKind::Unary(_, operand) => self.walk_expr(operand, row),
            ExprKind::Binary(_, l, r) => {
                self.walk_expr(l, row);
                self.walk_expr(r, row);
            }
            ExprKind::If { cond, then_block, else_branch } => {
                self.walk_expr(cond, row);
                self.walk_block(then_block, row);
                if let Some(else_e) = else_branch {
                    self.walk_expr(else_e, row);
                }
            }
            ExprKind::Block(block) => self.walk_block(block, row),
            ExprKind::List(elems) => {
                for el in elems {
                    self.walk_expr(el, row);
                }
            }
            ExprKind::Return(value) => {
                if let Some(v) = value {
                    self.walk_expr(v, row);
                }
            }
            ExprKind::Abort(value) => {
                if self.inline_op_depth == 0 {
                    self.report(
                        Diagnostic::error("E0409", "`abort` outside an inline handler operation")
                            .primary(e.span, "only valid directly inside `handle E with { fn op(..) { .. } }`"),
                    );
                }
                self.walk_expr(value, row);
            }
            ExprKind::Handle { bindings, body } => self.walk_handle(bindings, body, row),
        }
    }

    fn resolve_value(&mut self, id: &Ident) {
        let name = id.name.as_str();
        if self.local(name).is_some() || BUILTIN_VALUES.contains(&name) {
            return;
        }
        let d = if self.fns.contains_key(name) || Builtin::lookup(name).is_some() {
            Diagnostic::error("E0107", format!("using function `{name}` as a value is not supported yet"))
                .primary(id.span, "call it instead")
        } else if self.effects.root(name).is_some() {
            Diagnostic::error("E0417", format!("effect `{name}` is not a value"))
                .primary(id.span, "call one of its operations instead")
        } else {
            Diagnostic::error("E0101", format!("cannot find value `{name}` in this scope"))
                .primary(id.span, "not found")
        };
        self.report(d);
    }

    fn walk_call(&mut self, call: &Expr, callee: &Expr, args: &[Arg], row: &mut Row) {
        let Some(path) = flatten_path(callee) else {
            self.walk_expr(callee, row);
            self.report(
                Diagnostic::error("E0107", "calling this kind of expression is not supported yet")
                    .primary(callee.span, "only functions, built-ins, and effect operations can be called"),
            );
            return;
        };
        let first = path[0];
        let name = first.name.as_str();

        if self.local(name).is_some() {
            let what = if path.len() == 1 { "calling a local value" } else { "method calls" };
            self.report(
                Diagnostic::error("E0107", format!("{what} are not supported yet"))
                    .primary(callee.span, "planned for Phase 1"),
            );
            return;
        }

        if path.len() == 1 {
            if self.fns.contains_key(name) {
                let decl = self.fn_decl(name);
                let params: Vec<String> = decl.params.iter().map(|p| p.name.name.clone()).collect();
                self.check_args(&format!("function `{name}`"), &params, args, call.span);
                self.callees.insert(call.id, Callee::Fn(name.to_string()));
                for leaf in self.visible_row(name) {
                    add(row, leaf, Origin { span: call.span, via: Via::Fn(name.to_string()) });
                }
            } else if let Some(builtin) = Builtin::lookup(name) {
                let (min, max) = builtin.arity();
                if let Some(named) = args.iter().find_map(|a| a.name.as_ref()) {
                    self.report(
                        Diagnostic::error("E0103", format!("`{name}` does not take named arguments"))
                            .primary(named.span, "remove the name"),
                    );
                } else if args.len() < min || args.len() > max {
                    let expected = if min == max { min.to_string() } else { format!("{min} or {max}") };
                    self.report(
                        Diagnostic::error(
                            "E0102",
                            format!("`{name}` takes {expected} argument(s) but {} were given", args.len()),
                        )
                        .primary(call.span, "wrong number of arguments"),
                    );
                }
                self.callees.insert(call.id, Callee::Builtin(builtin));
            } else if self.effects.root(name).is_some() {
                self.report(
                    Diagnostic::error("E0404", format!("`{name}` is an effect, not a function"))
                        .primary(first.span, "call one of its operations, e.g. `console.print(..)`"),
                );
            } else {
                self.report(
                    Diagnostic::error("E0101", format!("cannot find function `{name}` in this scope"))
                        .primary(first.span, "not found"),
                );
            }
            return;
        }

        let Some(root) = self.effects.root(name) else {
            let msg = if self.fns.contains_key(name) || Builtin::lookup(name).is_some() {
                Diagnostic::error("E0107", "method calls are not supported yet")
                    .primary(callee.span, "planned for Phase 1")
            } else {
                Diagnostic::error("E0101", format!("cannot find value `{name}` in this scope"))
                    .primary(first.span, "not found")
            };
            self.report(msg);
            return;
        };

        // §5.2: follow nested effects as far as possible, leaving the last segment as the op name.
        let mut node = root;
        let mut i = 1;
        while i < path.len() - 1 {
            match self.effects.child(node, &path[i].name) {
                Some(c) => {
                    node = c;
                    i += 1;
                }
                None => break,
            }
        }
        let node_path = self.effects.nodes[node].path.clone();
        if path.len() - i != 1 {
            self.report(
                Diagnostic::error("E0404", format!("`{node_path}` has no nested effect `{}`", path[i].name))
                    .primary(path[i].span, "unknown effect segment"),
            );
            return;
        }
        let op_ident = path[i];
        match self.effects.resolve_op(node, &op_ident.name) {
            OpLookup::Found { leaf, op } => {
                let leaf_node = &self.effects.nodes[leaf];
                let leaf_path = leaf_node.path.clone();
                let params = leaf_node.ops[op].params.clone();
                let op_name = op_ident.name.clone();
                self.check_args(&format!("operation `{leaf_path}.{op_name}`"), &params, args, call.span);
                self.callees.insert(call.id, Callee::Op { leaf: leaf_path.clone(), op: op_name.clone() });
                let via = Via::Op(format!("{leaf_path}.{op_name}"));
                add(row, leaf_path, Origin { span: call.span, via });
            }
            OpLookup::Unknown => {
                let available: Vec<String> = self
                    .effects
                    .ops_in_subtree(node)
                    .iter()
                    .map(|&(l, o)| format!("`{}`", self.effects.nodes[l].ops[o].name))
                    .collect();
                self.report(
                    Diagnostic::error("E0404", format!("effect `{node_path}` has no operation `{}`", op_ident.name))
                        .primary(op_ident.span, "unknown operation")
                        .help(format!("available operations: {}", available.join(", "))),
                );
            }
            OpLookup::Ambiguous(paths) => {
                self.report(
                    Diagnostic::error(
                        "E0403",
                        format!("operation `{}` is ambiguous under `{node_path}`", op_ident.name),
                    )
                    .primary(op_ident.span, "matches more than one operation")
                    .help(format!("qualify it: {}", paths.join(", "))),
                );
            }
        }
    }

    fn check_args(&mut self, what: &str, params: &[String], args: &[Arg], call_span: Span) {
        let mut filled = vec![false; params.len()];
        let mut seen_named = false;
        let mut positional = 0;
        for arg in args {
            match &arg.name {
                None => {
                    if seen_named {
                        self.report(
                            Diagnostic::error("E0105", "positional argument after a named argument")
                                .primary(arg.value.span, "move positional arguments first"),
                        );
                        continue;
                    }
                    if positional < params.len() {
                        filled[positional] = true;
                    } else {
                        self.report(
                            Diagnostic::error(
                                "E0102",
                                format!("{what} takes {} argument(s) but more were given", params.len()),
                            )
                            .primary(arg.value.span, "unexpected argument"),
                        );
                    }
                    positional += 1;
                }
                Some(name) => {
                    seen_named = true;
                    match params.iter().position(|p| *p == name.name) {
                        None => self.report(
                            Diagnostic::error("E0103", format!("{what} has no parameter named `{}`", name.name))
                                .primary(name.span, "unknown parameter"),
                        ),
                        Some(i) if filled[i] => self.report(
                            Diagnostic::error("E0103", format!("argument `{}` is given more than once", name.name))
                                .primary(name.span, "duplicate argument"),
                        ),
                        Some(i) => filled[i] = true,
                    }
                }
            }
        }
        let missing: Vec<String> =
            params.iter().zip(&filled).filter(|(_, f)| !**f).map(|(p, _)| format!("`{p}`")).collect();
        if !missing.is_empty() {
            self.report(
                Diagnostic::error("E0102", format!("missing argument(s) {} for {what}", missing.join(", ")))
                    .primary(call_span, "in this call"),
            );
        }
    }

    fn walk_handle(&mut self, bindings: &[HandlerBinding], body: &Block, row: &mut Row) {
        let mut nodes = Vec::new();
        for b in bindings {
            let node = self.effects.lookup(&b.effect.dotted());
            match node {
                None => self.report(self.unknown_effect(&b.effect)),
                Some(n) => self.check_inline_ops(b, n),
            }
            nodes.push(node);
        }

        let mut body_row = Row::new();
        self.walk_block(body, &mut body_row);

        // `handle A with a, B with b { body }` ≡ `handle A with a { handle B with b { body } }`.
        for (b, node) in bindings.iter().zip(nodes).rev() {
            if let Some(n) = node {
                let leaves = self.effects.leaves(n);
                if !body_row.keys().any(|k| leaves.contains(k)) {
                    self.report(
                        Diagnostic::warning("W0403", format!("handler for `{}` is never used", b.effect.dotted()))
                            .primary(b.effect.span, "the body does not perform this effect"),
                    );
                }
                body_row.retain(|k, _| !leaves.contains(k));
            }
            // Op bodies run outside this handler (§6.5), so their effects reach outer handlers.
            for op in &b.ops {
                let params = op.params.iter().map(|p| (p.name.name.clone(), false)).collect();
                self.scopes.push(params);
                self.inline_op_depth += 1;
                self.walk_block(&op.body, &mut body_row);
                self.inline_op_depth -= 1;
                self.scopes.pop();
            }
        }
        for (leaf, origin) in body_row {
            add(row, leaf, origin);
        }
    }

    fn check_inline_ops(&mut self, b: &HandlerBinding, node: usize) {
        let expected: Vec<(String, usize)> = self
            .effects
            .ops_in_subtree(node)
            .into_iter()
            .map(|(l, o)| {
                let op = &self.effects.nodes[l].ops[o];
                (op.name.clone(), op.params.len())
            })
            .collect();
        let path = b.effect.dotted();
        let mut seen: Vec<&str> = Vec::new();
        for op in &b.ops {
            if seen.contains(&op.name.name.as_str()) {
                self.report(
                    Diagnostic::error("E0416", format!("operation `{}` is implemented twice", op.name.name))
                        .primary(op.name.span, "duplicate implementation"),
                );
                continue;
            }
            seen.push(&op.name.name);
            match expected.iter().find(|(n, _)| *n == op.name.name) {
                None => self.report(
                    Diagnostic::error("E0404", format!("effect `{path}` has no operation `{}`", op.name.name))
                        .primary(op.name.span, "not an operation of this effect"),
                ),
                Some((_, arity)) if *arity != op.params.len() => self.report(
                    Diagnostic::error(
                        "E0407",
                        format!(
                            "operation `{}` takes {arity} parameter(s) but this implementation has {}",
                            op.name.name,
                            op.params.len()
                        ),
                    )
                    .primary(op.name.span, "signature does not match the effect declaration"),
                ),
                Some(_) => {}
            }
        }
        let missing: Vec<String> =
            expected.iter().filter(|(n, _)| !seen.contains(&n.as_str())).map(|(n, _)| format!("`{n}`")).collect();
        if !missing.is_empty() {
            self.report(
                Diagnostic::error(
                    "E0406",
                    format!("handler for `{path}` is missing operation(s) {}", missing.join(", ")),
                )
                .primary(b.effect.span, "every operation of the effect must be implemented"),
            );
        }
    }
}

/// `a.b.c` as identifiers, if `e` is a plain dotted name.
fn flatten_path(e: &Expr) -> Option<Vec<&Ident>> {
    match &e.kind {
        ExprKind::Name(id) => Some(vec![id]),
        ExprKind::Field(base, name) => {
            let mut path = flatten_path(base)?;
            path.push(name);
            Some(path)
        }
        _ => None,
    }
}
