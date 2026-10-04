//! Tree-walking interpreter with deep, tail-resumptive effect handlers
//! (docs/spec/effects.md §8). Root handlers for built-in effects live here
//! until `nova_runtime` exists.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, Write};
use std::rc::Rc;

use nova_syntax::ast::*;
use nova_types::{Builtin, Callee, Program};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    List(Rc<Vec<Value>>),
    None,
    Some(Rc<Value>),
}

impl Value {
    fn str(s: impl Into<Rc<str>>) -> Value {
        Value::Str(s.into())
    }

    /// Source-like rendering used in assertion messages.
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => format!("{s:?}"),
            Value::List(items) => {
                format!("[{}]", items.iter().map(Value::repr).collect::<Vec<_>>().join(", "))
            }
            Value::Some(v) => format!("Some({})", v.repr()),
            other => other.to_string(),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Value::Unit => "()",
            Value::Bool(_) => "Bool",
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Str(_) => "String",
            Value::List(_) => "List",
            Value::None | Value::Some(_) => "Option",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) if x.fract() == 0.0 && x.is_finite() => write!(f, "{x:.1}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::List(_) | Value::Some(_) => write!(f, "{}", self.repr()),
            Value::None => write!(f, "None"),
        }
    }
}

/// Non-local control flow.
#[derive(Debug)]
pub enum Flow {
    Return(Value),
    Abort { handle: u64, value: Value },
    Panic(String),
}

type Eval = Result<Value, Flow>;

fn panic<T>(msg: impl Into<String>) -> Result<T, Flow> {
    Err(Flow::Panic(msg.into()))
}

struct Scope {
    vars: RefCell<HashMap<String, Rc<RefCell<Value>>>>,
    parent: Option<Rc<Scope>>,
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Scope { vars: RefCell::new(HashMap::new()), parent })
    }

    fn define(&self, name: &str, value: Value) {
        self.vars.borrow_mut().insert(name.to_string(), Rc::new(RefCell::new(value)));
    }

    fn lookup(&self, name: &str) -> Option<Rc<RefCell<Value>>> {
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref()?.lookup(name)
    }
}

enum HandlerKind<'p> {
    RootConsole,
    Inline { binding: &'p HandlerBinding, env: Rc<Scope>, handle: u64 },
}

/// One installed handler. Frames form a linked stack so an operation body can
/// run with only the frames *below* its handler visible (delegation, §6.5).
struct Frame<'p> {
    node: String,
    kind: HandlerKind<'p>,
    parent: Option<Rc<Frame<'p>>>,
}

pub struct Interp<'p> {
    program: &'p Program,
    handlers: Option<Rc<Frame<'p>>>,
    /// Targets for `abort` inside the inline op bodies currently executing.
    abort_targets: Vec<u64>,
    next_handle: u64,
}

impl<'p> Interp<'p> {
    /// An interpreter with root handlers installed for the given built-in leaves.
    pub fn new(program: &'p Program, root_leaves: &[&str]) -> Interp<'p> {
        let mut handlers = None;
        for &leaf in root_leaves {
            let kind = match leaf {
                "console" => HandlerKind::RootConsole,
                other => panic!("no root handler implemented for `{other}`"),
            };
            handlers = Some(Rc::new(Frame { node: leaf.to_string(), kind, parent: handlers }));
        }
        Interp { program, handlers, abort_targets: Vec::new(), next_handle: 0 }
    }

    /// Calls `main` with no arguments.
    pub fn run_main(&mut self) -> Result<Value, String> {
        let main = self.program.fn_decl("main").ok_or("no `main` function")?;
        let result = self.call_fn(main, Vec::new());
        finish(result)
    }

    /// Runs a `test` body.
    pub fn run_test(&mut self, test: &'p TestDecl) -> Result<(), String> {
        let scope = Scope::new(None);
        let result = self.eval_block(&test.body, &scope);
        finish(result).map(|_| ())
    }

    fn call_fn(&mut self, decl: &'p FnDecl, args: Vec<Value>) -> Eval {
        let scope = Scope::new(None);
        for (param, value) in decl.params.iter().zip(args) {
            scope.define(&param.name.name, value);
        }
        match self.eval_block(&decl.body, &scope) {
            Err(Flow::Return(v)) => Ok(v),
            other => other,
        }
    }

    fn eval_block(&mut self, block: &'p Block, parent: &Rc<Scope>) -> Eval {
        let scope = Scope::new(Some(parent.clone()));
        let mut last = Value::Unit;
        for stmt in &block.stmts {
            last = Value::Unit;
            match stmt {
                Stmt::Let { name, value, .. } => {
                    let v = self.eval(value, &scope)?;
                    scope.define(&name.name, v);
                }
                Stmt::Assign { target, op, value, .. } => {
                    let v = self.eval(value, &scope)?;
                    let Some(cell) = scope.lookup(&target.name) else {
                        return panic(format!("internal error: unbound variable `{}`", target.name));
                    };
                    let new = match op {
                        AssignOp::Set => v,
                        AssignOp::Add => binary(BinOp::Add, cell.borrow().clone(), v)?,
                        AssignOp::Sub => binary(BinOp::Sub, cell.borrow().clone(), v)?,
                    };
                    *cell.borrow_mut() = new;
                }
                Stmt::Expr(e) => last = self.eval(e, &scope)?,
            }
        }
        Ok(last)
    }

    fn eval(&mut self, e: &'p Expr, scope: &Rc<Scope>) -> Eval {
        match &e.kind {
            ExprKind::Int(n) => Ok(Value::Int(*n)),
            ExprKind::Float(x) => Ok(Value::Float(*x)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Str(segs) => {
                let mut s = String::new();
                for seg in segs {
                    match seg {
                        StrSeg::Text(t) => s.push_str(t),
                        StrSeg::Expr(inner) => s.push_str(&self.eval(inner, scope)?.to_string()),
                    }
                }
                Ok(Value::str(s))
            }
            ExprKind::Name(id) => match scope.lookup(&id.name) {
                Some(cell) => Ok(cell.borrow().clone()),
                None if id.name == "None" => Ok(Value::None),
                None => panic(format!("internal error: unbound name `{}`", id.name)),
            },
            ExprKind::Call { args, .. } => self.eval_call(e, args, scope),
            ExprKind::Unary(op, operand) => {
                let v = self.eval(operand, scope)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(n)) => {
                        n.checked_neg().map(Value::Int).map_or_else(|| panic("integer overflow"), Ok)
                    }
                    (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (op, v) => panic(format!(
                        "type error: cannot apply `{}` to {}",
                        if *op == UnOp::Neg { "-" } else { "!" },
                        v.type_name()
                    )),
                }
            }
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), l, r) => {
                let Value::Bool(lv) = self.eval(l, scope)? else {
                    return panic(format!("type error: `{}` needs Bool operands", op.as_str()));
                };
                if (*op == BinOp::And && !lv) || (*op == BinOp::Or && lv) {
                    return Ok(Value::Bool(lv));
                }
                match self.eval(r, scope)? {
                    Value::Bool(rv) => Ok(Value::Bool(rv)),
                    _ => panic(format!("type error: `{}` needs Bool operands", op.as_str())),
                }
            }
            ExprKind::Binary(op, l, r) => {
                let lv = self.eval(l, scope)?;
                let rv = self.eval(r, scope)?;
                binary(*op, lv, rv)
            }
            ExprKind::If { cond, then_block, else_branch } => match self.eval(cond, scope)? {
                Value::Bool(true) => self.eval_block(then_block, scope),
                Value::Bool(false) => match else_branch {
                    Some(else_e) => self.eval(else_e, scope),
                    None => Ok(Value::Unit),
                },
                other => panic(format!("type error: `if` condition must be Bool, found {}", other.type_name())),
            },
            ExprKind::Block(block) => self.eval_block(block, scope),
            ExprKind::List(elems) => {
                let mut items = Vec::with_capacity(elems.len());
                for el in elems {
                    items.push(self.eval(el, scope)?);
                }
                Ok(Value::List(Rc::new(items)))
            }
            ExprKind::Return(value) => {
                let v = match value {
                    Some(v) => self.eval(v, scope)?,
                    None => Value::Unit,
                };
                Err(Flow::Return(v))
            }
            ExprKind::Abort(value) => {
                let v = self.eval(value, scope)?;
                let Some(&handle) = self.abort_targets.last() else {
                    return panic("internal error: `abort` outside a handler");
                };
                Err(Flow::Abort { handle, value: v })
            }
            ExprKind::Handle { bindings, body } => self.eval_handle(bindings, body, scope),
            ExprKind::Field(..) | ExprKind::Error => {
                panic("internal error: unsupported expression reached the interpreter")
            }
        }
    }

    fn eval_call(&mut self, call: &'p Expr, args: &'p [Arg], scope: &Rc<Scope>) -> Eval {
        let Some(callee) = self.program.callees.get(&call.id) else {
            return panic("internal error: unresolved call");
        };
        match callee {
            Callee::Fn(name) => {
                let decl = self.program.fn_decl(name).expect("checked");
                let params: Vec<&str> = decl.params.iter().map(|p| p.name.name.as_str()).collect();
                let values = self.bind_args(&params, args, scope)?;
                self.call_fn(decl, values)
            }
            Callee::Builtin(builtin) => {
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval(&arg.value, scope)?);
                }
                call_builtin(*builtin, values)
            }
            Callee::Op { leaf, op } => {
                let node = self.program.effects.lookup(leaf).expect("checked");
                let info = self.program.effects.nodes[node].ops.iter().find(|o| o.name == *op).expect("checked");
                let params: Vec<&str> = info.params.iter().map(String::as_str).collect();
                let values = self.bind_args(&params, args, scope)?;
                self.perform(leaf, op, values)
            }
        }
    }

    /// Evaluates arguments left to right and orders them by parameter.
    fn bind_args(&mut self, params: &[&str], args: &'p [Arg], scope: &Rc<Scope>) -> Result<Vec<Value>, Flow> {
        let mut slots: Vec<Option<Value>> = vec![None; params.len()];
        for (i, arg) in args.iter().enumerate() {
            let v = self.eval(&arg.value, scope)?;
            let index = match &arg.name {
                Some(name) => params.iter().position(|p| *p == name.name).expect("checked"),
                None => i,
            };
            slots[index] = Some(v);
        }
        Ok(slots.into_iter().map(|v| v.expect("checked")).collect())
    }

    fn perform(&mut self, leaf: &str, op: &str, args: Vec<Value>) -> Eval {
        let mut cursor = self.handlers.clone();
        while let Some(frame) = cursor {
            if leaf == frame.node || leaf.starts_with(&format!("{}.", frame.node)) {
                // Run the op with only the frames below this handler visible.
                let saved = std::mem::replace(&mut self.handlers, frame.parent.clone());
                let result = self.run_op(&frame, op, args);
                self.handlers = saved;
                return result;
            }
            cursor = frame.parent.clone();
        }
        panic(format!("internal error: no handler for `{leaf}.{op}`"))
    }

    fn run_op(&mut self, frame: &Frame<'p>, op: &str, args: Vec<Value>) -> Eval {
        match &frame.kind {
            HandlerKind::RootConsole => root_console(op, args),
            HandlerKind::Inline { binding, env, handle } => {
                let op_impl = binding.ops.iter().find(|o| o.name.name == op).expect("checked");
                let scope = Scope::new(Some(env.clone()));
                for (param, value) in op_impl.params.iter().zip(args) {
                    scope.define(&param.name.name, value);
                }
                self.abort_targets.push(*handle);
                let result = self.eval_block(&op_impl.body, &scope);
                self.abort_targets.pop();
                match result {
                    Err(Flow::Return(v)) => Ok(v),
                    other => other,
                }
            }
        }
    }

    fn eval_handle(&mut self, bindings: &'p [HandlerBinding], body: &'p Block, scope: &Rc<Scope>) -> Eval {
        let handle = self.next_handle;
        self.next_handle += 1;
        let saved = self.handlers.clone();
        for binding in bindings {
            let kind = HandlerKind::Inline { binding, env: scope.clone(), handle };
            self.handlers = Some(Rc::new(Frame { node: binding.effect.dotted(), kind, parent: self.handlers.take() }));
        }
        let result = self.eval_block(body, scope);
        self.handlers = saved;
        match result {
            Err(Flow::Abort { handle: h, value }) if h == handle => Ok(value),
            other => other,
        }
    }
}

fn finish(result: Eval) -> Result<Value, String> {
    match result {
        Ok(v) | Err(Flow::Return(v)) => Ok(v),
        Err(Flow::Panic(msg)) => Err(msg),
        Err(Flow::Abort { .. }) => Err("internal error: `abort` escaped its handler".into()),
    }
}

fn root_console(op: &str, args: Vec<Value>) -> Eval {
    match op {
        "print" => {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{}", args[0]);
            let _ = out.flush();
            Ok(Value::Unit)
        }
        "eprint" => {
            eprintln!("{}", args[0]);
            Ok(Value::Unit)
        }
        "read_line" => {
            let mut line = String::new();
            match std::io::stdin().lock().read_line(&mut line) {
                Ok(0) | Err(_) => Ok(Value::None),
                Ok(_) => Ok(Value::Some(Rc::new(Value::str(line.trim_end_matches(['\n', '\r']))))),
            }
        }
        other => panic(format!("internal error: console has no operation `{other}`")),
    }
}

fn call_builtin(builtin: Builtin, args: Vec<Value>) -> Eval {
    match builtin {
        Builtin::Assert => match &args[0] {
            Value::Bool(true) => Ok(Value::Unit),
            Value::Bool(false) => match args.get(1) {
                Some(msg) => panic(format!("assertion failed: {msg}")),
                None => panic("assertion failed"),
            },
            other => panic(format!("type error: `assert` needs a Bool, found {}", other.type_name())),
        },
        Builtin::AssertEq => {
            if args[0] == args[1] {
                return Ok(Value::Unit);
            }
            let context = args.get(2).map(|m| format!(": {m}")).unwrap_or_default();
            panic(format!(
                "assertion failed: left == right{context}\n  left: {}\n right: {}",
                args[0].repr(),
                args[1].repr()
            ))
        }
        Builtin::Panic => panic(args[0].to_string()),
        Builtin::Some => Ok(Value::Some(Rc::new(args.into_iter().next().expect("checked")))),
    }
}

fn binary(op: BinOp, l: Value, r: Value) -> Eval {
    use Value::*;
    let overflow = || Flow::Panic("integer overflow".into());
    Ok(match (op, &l, &r) {
        (BinOp::Eq, _, _) => Bool(l == r),
        (BinOp::Ne, _, _) => Bool(l != r),
        (BinOp::Add, Int(a), Int(b)) => Int(a.checked_add(*b).ok_or_else(overflow)?),
        (BinOp::Sub, Int(a), Int(b)) => Int(a.checked_sub(*b).ok_or_else(overflow)?),
        (BinOp::Mul, Int(a), Int(b)) => Int(a.checked_mul(*b).ok_or_else(overflow)?),
        (BinOp::Div | BinOp::Rem, Int(_), Int(0)) => return panic("division by zero"),
        (BinOp::Div, Int(a), Int(b)) => Int(a.checked_div(*b).ok_or_else(overflow)?),
        (BinOp::Rem, Int(a), Int(b)) => Int(a.checked_rem(*b).ok_or_else(overflow)?),
        (BinOp::Add, Float(a), Float(b)) => Float(a + b),
        (BinOp::Sub, Float(a), Float(b)) => Float(a - b),
        (BinOp::Mul, Float(a), Float(b)) => Float(a * b),
        (BinOp::Div, Float(a), Float(b)) => Float(a / b),
        (BinOp::Rem, Float(a), Float(b)) => Float(a % b),
        (BinOp::Add, Str(a), Str(b)) => Value::str(format!("{a}{b}")),
        (BinOp::Lt, Int(a), Int(b)) => Bool(a < b),
        (BinOp::Gt, Int(a), Int(b)) => Bool(a > b),
        (BinOp::Le, Int(a), Int(b)) => Bool(a <= b),
        (BinOp::Ge, Int(a), Int(b)) => Bool(a >= b),
        (BinOp::Lt, Float(a), Float(b)) => Bool(a < b),
        (BinOp::Gt, Float(a), Float(b)) => Bool(a > b),
        (BinOp::Le, Float(a), Float(b)) => Bool(a <= b),
        (BinOp::Ge, Float(a), Float(b)) => Bool(a >= b),
        (BinOp::Lt, Str(a), Str(b)) => Bool(a < b),
        (BinOp::Gt, Str(a), Str(b)) => Bool(a > b),
        (BinOp::Le, Str(a), Str(b)) => Bool(a <= b),
        (BinOp::Ge, Str(a), Str(b)) => Bool(a >= b),
        _ => {
            return panic(format!(
                "type error: cannot apply `{}` to {} and {}",
                op.as_str(),
                l.type_name(),
                r.type_name()
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_diag::SourceMap;
    use nova_types::{RUNTIME_ROOT_LEAVES, compile};

    fn program(src: &str) -> Program {
        let mut sources = SourceMap::default();
        let (program, diags) = compile(&mut sources, "t.nova", src);
        assert!(!diags.iter().any(|d| d.is_error()), "{diags:#?}");
        program.unwrap()
    }

    fn run_tests(src: &str) -> Vec<(String, Result<(), String>)> {
        let p = program(src);
        p.tests().map(|t| (t.name.clone(), Interp::new(&p, RUNTIME_ROOT_LEAVES).run_test(t))).collect()
    }

    #[test]
    fn arithmetic_strings_and_control_flow() {
        let results = run_tests(
            r#"
fn fact(n: Int) -> Int {
    if n <= 1 {
        return 1
    }
    n * fact(n - 1)
}

test "math" {
    assert_eq(fact(5), 120)
    assert_eq("a{1 + 1}b", "a2b")
    var total = 0
    total += 5
    assert(total == 5 && !false)
}
"#,
        );
        assert_eq!(results[0].1, Ok(()));
    }

    #[test]
    fn handler_captures_and_delegates() {
        let results = run_tests(
            r#"
fn greet(name: String) uses [console] {
    console.print("hi " + name)
}

test "capture" {
    var out = ""
    handle console with {
        fn print(text) { out = out + "[" + text + "]" }
        fn eprint(text) { }
        fn read_line() { None }
    } {
        handle console with {
            fn print(text) { console.print(text + "!") }
            fn eprint(text) { }
            fn read_line() { None }
        } {
            greet("bob")
        }
    }
    assert_eq(out, "[hi bob!]")
}
"#,
        );
        assert_eq!(results[0].1, Ok(()));
    }

    #[test]
    fn abort_returns_from_handle() {
        let results = run_tests(
            r#"
test "abort" {
    let r = handle console with {
        fn print(text) { abort "stopped at " + text }
        fn eprint(text) { }
        fn read_line() { None }
    } {
        console.print("x")
        "unreachable"
    }
    assert_eq(r, "stopped at x")
}
"#,
        );
        assert_eq!(results[0].1, Ok(()));
    }

    #[test]
    fn failing_assertion_reports_values() {
        let results = run_tests("test \"bad\" {\n    assert_eq(1 + 1, 3)\n}\n");
        assert_eq!(results[0].1, Err("assertion failed: left == right\n  left: 2\n right: 3".into()));
    }

    #[test]
    fn overflow_panics() {
        let results = run_tests("test \"of\" {\n    let x = 9223372036854775807 + 1\n}\n");
        assert_eq!(results[0].1, Err("integer overflow".into()));
    }
}
