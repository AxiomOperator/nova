//! Recursive-descent parser with Pratt expression parsing.
//!
//! Newline handling (provisional, pending `docs/spec/syntax.md`):
//! - a newline ends a statement;
//! - newlines are ignored inside `()` and `[]`, after a binary operator,
//!   and before a line that starts with `.`;
//! - a function's `uses` clause and body may start on following lines.

use crate::ast::*;
use crate::lexer::{self, Kw, Punct, StrPiece, Tok, Token};
use nova_diag::{Diagnostic, FileId, Span};

/// Parses one source file. Always returns a (possibly partial) tree.
pub fn parse_file(file: FileId, src: &str) -> (File, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let toks = lexer::lex(file, src, 0, &mut diags);
    let mut p = Parser { toks, pos: 0, file, next_id: 0, prev_span: Span::new(file, 0, 0), diags };
    let file_ast = p.file();
    (file_ast, p.diags)
}

/// Marker for a parse failure whose diagnostic has already been recorded.
struct Fail;
type PResult<T> = Result<T, Fail>;

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    file: FileId,
    next_id: u32,
    prev_span: Span,
    diags: Vec<Diagnostic>,
}

fn is_item_start(tok: &Tok) -> bool {
    matches!(
        tok,
        Tok::Kw(
            Kw::Fn
                | Kw::Pub
                | Kw::Effect
                | Kw::Test
                | Kw::Type
                | Kw::Enum
                | Kw::Trait
                | Kw::Impl
                | Kw::Handler
                | Kw::Use
                | Kw::Tool
                | Kw::Model
                | Kw::Eval
        ) | Tok::P(Punct::At)
    )
}

fn infix(tok: &Tok) -> Option<(BinOp, u8)> {
    let Tok::P(p) = tok else { return None };
    Some(match p {
        Punct::OrOr => (BinOp::Or, 1),
        Punct::AndAnd => (BinOp::And, 2),
        Punct::EqEq => (BinOp::Eq, 3),
        Punct::Ne => (BinOp::Ne, 3),
        Punct::Lt => (BinOp::Lt, 4),
        Punct::Gt => (BinOp::Gt, 4),
        Punct::Le => (BinOp::Le, 4),
        Punct::Ge => (BinOp::Ge, 4),
        Punct::Plus => (BinOp::Add, 5),
        Punct::Minus => (BinOp::Sub, 5),
        Punct::Star => (BinOp::Mul, 6),
        Punct::Slash => (BinOp::Div, 6),
        Punct::Percent => (BinOp::Rem, 6),
        _ => return None,
    })
}

impl Parser {
    // ---- token helpers ----

    fn tok(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn nth(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if t.tok != Tok::Eof {
            self.pos += 1;
        }
        self.prev_span = t.span;
        t
    }

    fn at(&self, p: Punct) -> bool {
        *self.tok() == Tok::P(p)
    }

    fn at_kw(&self, kw: Kw) -> bool {
        *self.tok() == Tok::Kw(kw)
    }

    fn eat(&mut self, p: Punct) -> bool {
        if self.at(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn skip_newlines(&mut self) {
        while *self.tok() == Tok::Newline {
            self.bump();
        }
    }

    fn peek_past_newlines(&self) -> &Tok {
        let mut i = self.pos;
        while self.toks[i].tok == Tok::Newline {
            i += 1;
        }
        &self.toks[i].tok
    }

    fn fail(&mut self, code: &'static str, message: impl Into<String>, label: impl Into<String>) -> Fail {
        let span = self.span();
        self.diags.push(Diagnostic::error(code, message).primary(span, label));
        Fail
    }

    fn expected(&mut self, what: &str) -> Fail {
        let found = self.tok().describe();
        self.fail("E0006", format!("expected {what}, found {found}"), format!("expected {what}"))
    }

    fn expect(&mut self, p: Punct, context: &str) -> PResult<Span> {
        if self.at(p) { Ok(self.bump().span) } else { Err(self.expected(&format!("`{}` {context}", p.as_str()))) }
    }

    fn ident(&mut self, what: &str) -> PResult<Ident> {
        if let Tok::Ident(name) = self.tok() {
            let name = name.clone();
            let span = self.bump().span;
            Ok(Ident { name, span })
        } else {
            Err(self.expected(what))
        }
    }

    fn mk(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = ExprId { file: self.file, index: self.next_id };
        self.next_id += 1;
        Expr { id, kind, span }
    }

    fn unsupported(&mut self, what: &str, planned: &str) -> Fail {
        self.fail("E0009", format!("{what} not supported yet"), format!("planned for {planned}"))
    }

    // ---- recovery ----

    /// Skips to the next line (at bracket depth 0) that starts an item.
    fn recover_item(&mut self) {
        let mut depth = 0i32;
        let start = self.pos;
        loop {
            match self.tok() {
                Tok::Eof => return,
                Tok::P(Punct::LBrace | Punct::LParen | Punct::LBracket) => depth += 1,
                Tok::P(Punct::RBrace | Punct::RParen | Punct::RBracket) => depth -= 1,
                Tok::Newline
                    if depth <= 0
                        && self.pos > start
                        && (is_item_start(self.peek_past_newlines()) || *self.peek_past_newlines() == Tok::Eof) =>
                {
                    return;
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// Skips to the end of the current statement, leaving a closing `}` in place.
    fn recover_stmt(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.tok() {
                Tok::Eof => return,
                Tok::Newline if depth <= 0 => return,
                Tok::P(Punct::RBrace) if depth <= 0 => return,
                Tok::P(Punct::LBrace | Punct::LParen | Punct::LBracket) => depth += 1,
                Tok::P(Punct::RBrace | Punct::RParen | Punct::RBracket) => depth -= 1,
                _ => {}
            }
            self.bump();
        }
    }

    // ---- items ----

    fn file(&mut self) -> File {
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if *self.tok() == Tok::Eof {
                break;
            }
            match self.item() {
                Ok(item) => items.push(item),
                Err(Fail) => self.recover_item(),
            }
        }
        File { id: self.file, items }
    }

    fn item(&mut self) -> PResult<Item> {
        let start = self.span();
        let is_pub = if self.at_kw(Kw::Pub) {
            self.bump();
            true
        } else {
            false
        };
        match self.tok().clone() {
            Tok::Kw(Kw::Fn) => self.fn_decl(is_pub, start).map(Item::Fn),
            Tok::Kw(Kw::Effect) => self.effect_decl(is_pub, start).map(Item::Effect),
            Tok::Kw(Kw::Test) if !is_pub => self.test_decl(start).map(Item::Test),
            Tok::Kw(
                kw @ (Kw::Type
                | Kw::Enum
                | Kw::Trait
                | Kw::Impl
                | Kw::Handler
                | Kw::Use
                | Kw::Tool
                | Kw::Model
                | Kw::Eval),
            ) => Err(self.unsupported(&format!("`{}` items are", kw.as_str()), "Phase 0 (P0-11..P0-14)")),
            Tok::P(Punct::At) => Err(self.unsupported("attributes are", "Phase 0 (P0-09)")),
            _ => {
                let found = self.tok().describe();
                Err(self.fail(
                    "E0008",
                    format!("expected an item, found {found}"),
                    "expected `fn`, `effect`, or `test`",
                ))
            }
        }
    }

    fn fn_decl(&mut self, is_pub: bool, start: Span) -> PResult<FnDecl> {
        self.bump(); // fn
        let name = self.ident("function name")?;
        let params = self.param_list(true)?;
        let ret = if self.eat(Punct::Arrow) { Some(self.type_expr()?) } else { None };
        let uses = if *self.peek_past_newlines() == Tok::Kw(Kw::Uses) {
            self.skip_newlines();
            Some(self.uses_clause()?)
        } else {
            None
        };
        if *self.peek_past_newlines() == Tok::P(Punct::LBrace) {
            self.skip_newlines();
        }
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(FnDecl { is_pub, name, params, ret, uses, body, span })
    }

    fn param_list(&mut self, require_types: bool) -> PResult<Vec<Param>> {
        self.expect(Punct::LParen, "to start the parameter list")?;
        let mut params = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(Punct::RParen) {
                break;
            }
            let name = self.ident("parameter name")?;
            let ty = if self.eat(Punct::Colon) {
                Some(self.type_expr()?)
            } else {
                if require_types {
                    self.diags.push(
                        Diagnostic::error("E0010", format!("missing type for parameter `{}`", name.name))
                            .primary(name.span, "add a type, e.g. `name: String`"),
                    );
                }
                None
            };
            params.push(Param { name, ty });
            self.skip_newlines();
            if !self.eat(Punct::Comma) {
                self.skip_newlines();
                self.expect(Punct::RParen, "to close the parameter list")?;
                break;
            }
        }
        Ok(params)
    }

    fn type_expr(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        let mut ty = if self.eat(Punct::LParen) {
            let mut elems = Vec::new();
            loop {
                self.skip_newlines();
                if self.eat(Punct::RParen) {
                    break;
                }
                elems.push(self.type_expr()?);
                self.skip_newlines();
                if !self.eat(Punct::Comma) {
                    self.expect(Punct::RParen, "to close the tuple type")?;
                    break;
                }
            }
            TypeExpr { kind: TypeKind::Tuple(elems), span: start.to(self.prev_span) }
        } else if self.at_kw(Kw::Fn) {
            return Err(self.unsupported("function types are", "Phase 0 (P0-09)"));
        } else {
            let mut path = vec![self.ident("type")?];
            while self.at(Punct::Dot) && matches!(self.nth(1), Tok::Ident(_)) {
                self.bump();
                path.push(self.ident("type name")?);
            }
            let mut args = Vec::new();
            if self.eat(Punct::Lt) {
                loop {
                    args.push(self.type_expr()?);
                    if !self.eat(Punct::Comma) {
                        self.expect(Punct::Gt, "to close the type arguments")?;
                        break;
                    }
                }
            }
            TypeExpr { kind: TypeKind::Path { path, args }, span: start.to(self.prev_span) }
        };
        while self.at(Punct::Question) {
            let end = self.bump().span;
            let span = ty.span.to(end);
            ty = TypeExpr { kind: TypeKind::Optional(Box::new(ty)), span };
        }
        Ok(ty)
    }

    fn uses_clause(&mut self) -> PResult<UsesClause> {
        let start = self.bump().span; // uses
        self.expect(Punct::LBracket, "after `uses`")?;
        let mut paths = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(Punct::RBracket) {
                break;
            }
            paths.push(self.effect_path()?);
            self.skip_newlines();
            if !self.eat(Punct::Comma) {
                self.skip_newlines();
                self.expect(Punct::RBracket, "to close the `uses` list")?;
                break;
            }
        }
        Ok(UsesClause { paths, span: start.to(self.prev_span) })
    }

    fn effect_path(&mut self) -> PResult<EffectPath> {
        let first = self.ident("effect name")?;
        let mut span = first.span;
        let mut segments = vec![first];
        while self.at(Punct::Dot) && matches!(self.nth(1), Tok::Ident(_)) {
            self.bump();
            let seg = self.ident("effect name")?;
            span = span.to(seg.span);
            segments.push(seg);
        }
        Ok(EffectPath { segments, span })
    }

    fn effect_decl(&mut self, is_pub: bool, start: Span) -> PResult<EffectDecl> {
        self.bump(); // effect
        let name = self.ident("effect name")?;
        self.expect(Punct::LBrace, "to start the effect body")?;
        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(Punct::RBrace) {
                break;
            }
            if *self.tok() == Tok::Eof {
                return Err(self.expected("`}` to close the effect"));
            }
            let member_start = self.span();
            let member = if self.at_kw(Kw::Effect) {
                self.effect_decl(false, member_start).map(EffectMember::Effect)
            } else if self.at_kw(Kw::Fn) {
                self.op_decl(member_start).map(EffectMember::Op)
            } else {
                let found = self.tok().describe();
                Err(self.fail(
                    "E0008",
                    format!("expected `fn` or `effect`, found {found}"),
                    "not valid in an effect body",
                ))
            };
            match member {
                Ok(m) => members.push(m),
                Err(Fail) => self.recover_stmt(),
            }
        }
        Ok(EffectDecl { is_pub, name, members, span: start.to(self.prev_span) })
    }

    fn op_decl(&mut self, start: Span) -> PResult<OpDecl> {
        self.bump(); // fn
        let name = self.ident("operation name")?;
        let params = self.param_list(true)?;
        let ret = if self.eat(Punct::Arrow) { Some(self.type_expr()?) } else { None };
        Ok(OpDecl { name, params, ret, span: start.to(self.prev_span) })
    }

    fn test_decl(&mut self, start: Span) -> PResult<TestDecl> {
        self.bump(); // test
        let name_span = self.span();
        let name = match self.tok().clone() {
            Tok::Str(pieces) => match pieces.as_slice() {
                [StrPiece::Text(text)] => {
                    self.bump();
                    text.clone()
                }
                _ => {
                    return Err(self.fail(
                        "E0011",
                        "test names must be plain string literals",
                        "no interpolation here",
                    ));
                }
            },
            _ => return Err(self.expected("test name string")),
        };
        if *self.peek_past_newlines() == Tok::P(Punct::LBrace) {
            self.skip_newlines();
        }
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(TestDecl { name, name_span, body, span })
    }

    // ---- statements ----

    fn block(&mut self) -> PResult<Block> {
        let start = self.expect(Punct::LBrace, "to start a block")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(Punct::RBrace) {
                let end = self.bump().span;
                return Ok(Block { stmts, span: start.to(end) });
            }
            if *self.tok() == Tok::Eof {
                return Err(self.expected("`}` to close the block"));
            }
            match self.stmt() {
                Ok(stmt) => {
                    stmts.push(stmt);
                    if !matches!(self.tok(), Tok::Newline | Tok::Eof | Tok::P(Punct::RBrace)) {
                        let found = self.tok().describe();
                        let _ = self.fail(
                            "E0012",
                            format!("expected newline or `}}` after statement, found {found}"),
                            "statements are separated by newlines",
                        );
                        self.recover_stmt();
                    }
                }
                Err(Fail) => self.recover_stmt(),
            }
        }
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        if self.at_kw(Kw::Let) || self.at_kw(Kw::Var) {
            let start = self.span();
            let mutable = self.at_kw(Kw::Var);
            self.bump();
            let name = self.ident("variable name")?;
            let ty = if self.eat(Punct::Colon) { Some(self.type_expr()?) } else { None };
            self.expect(Punct::Eq, "in variable binding")?;
            self.skip_newlines();
            let value = self.expr()?;
            let span = start.to(value.span);
            return Ok(Stmt::Let { mutable, name, ty, value, span });
        }
        let e = self.expr()?;
        let op = match self.tok() {
            Tok::P(Punct::Eq) => AssignOp::Set,
            Tok::P(Punct::PlusEq) => AssignOp::Add,
            Tok::P(Punct::MinusEq) => AssignOp::Sub,
            _ => return Ok(Stmt::Expr(e)),
        };
        let ExprKind::Name(target) = e.kind else {
            return Err(self.fail(
                "E0013",
                "invalid assignment target",
                "only variables can be assigned in this build",
            ));
        };
        self.bump();
        self.skip_newlines();
        let value = self.expr()?;
        let span = e.span.to(value.span);
        Ok(Stmt::Assign { target, op, value, span })
    }

    // ---- expressions ----

    fn expr(&mut self) -> PResult<Expr> {
        self.expr_bp(0)
    }

    fn expr_bp(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = infix(self.tok()) {
            if prec < min_prec {
                break;
            }
            self.bump();
            self.skip_newlines();
            let rhs = self.expr_bp(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = self.mk(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let op = match self.tok() {
            Tok::P(Punct::Minus) => UnOp::Neg,
            Tok::P(Punct::Bang) => UnOp::Not,
            _ => return self.postfix(),
        };
        let start = self.bump().span;
        let operand = self.unary()?;
        let span = start.to(operand.span);
        Ok(self.mk(ExprKind::Unary(op, Box::new(operand)), span))
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.at(Punct::LParen) {
                let args = self.args()?;
                let span = e.span.to(self.prev_span);
                e = self.mk(ExprKind::Call { callee: Box::new(e), args }, span);
            } else if self.at(Punct::Dot) {
                self.bump();
                let name = self.ident("field or method name")?;
                let span = e.span.to(name.span);
                e = self.mk(ExprKind::Field(Box::new(e), name), span);
            } else if *self.tok() == Tok::Newline && *self.peek_past_newlines() == Tok::P(Punct::Dot) {
                self.skip_newlines();
            } else if self.at(Punct::Question) {
                return Err(self.unsupported("the `?` operator is", "Phase 1 (Result types)"));
            } else {
                return Ok(e);
            }
        }
    }

    fn args(&mut self) -> PResult<Vec<Arg>> {
        self.bump(); // (
        let mut args = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(Punct::RParen) {
                break;
            }
            let name = if matches!(self.tok(), Tok::Ident(_)) && *self.nth(1) == Tok::P(Punct::Colon) {
                let name = self.ident("argument name")?;
                self.bump(); // :
                self.skip_newlines();
                Some(name)
            } else {
                None
            };
            let value = self.expr()?;
            args.push(Arg { name, value });
            self.skip_newlines();
            if !self.eat(Punct::Comma) {
                self.skip_newlines();
                self.expect(Punct::RParen, "to close the argument list")?;
                break;
            }
        }
        Ok(args)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        match self.tok().clone() {
            Tok::Int(n) => {
                self.bump();
                Ok(self.mk(ExprKind::Int(n), start))
            }
            Tok::Float(f) => {
                self.bump();
                Ok(self.mk(ExprKind::Float(f), start))
            }
            Tok::Kw(Kw::True) | Tok::Kw(Kw::False) => {
                let b = self.at_kw(Kw::True);
                self.bump();
                Ok(self.mk(ExprKind::Bool(b), start))
            }
            Tok::Str(pieces) => {
                self.bump();
                let mut segs = Vec::new();
                for piece in pieces {
                    match piece {
                        StrPiece::Text(t) => segs.push(StrSeg::Text(t)),
                        StrPiece::Interp(tokens) => segs.push(StrSeg::Expr(self.interpolation(tokens, start)?)),
                    }
                }
                Ok(self.mk(ExprKind::Str(segs), start))
            }
            Tok::Ident(name) => {
                self.bump();
                Ok(self.mk(ExprKind::Name(Ident { name, span: start }), start))
            }
            Tok::P(Punct::LParen) => {
                self.bump();
                self.skip_newlines();
                if self.eat(Punct::RParen) {
                    let span = start.to(self.prev_span);
                    return Ok(self.mk(ExprKind::Block(Block { stmts: Vec::new(), span }), span));
                }
                let e = self.expr()?;
                self.skip_newlines();
                self.expect(Punct::RParen, "to close the parenthesized expression")?;
                Ok(e)
            }
            Tok::P(Punct::LBracket) => {
                self.bump();
                let mut elems = Vec::new();
                loop {
                    self.skip_newlines();
                    if self.eat(Punct::RBracket) {
                        break;
                    }
                    elems.push(self.expr()?);
                    self.skip_newlines();
                    if !self.eat(Punct::Comma) {
                        self.skip_newlines();
                        self.expect(Punct::RBracket, "to close the list")?;
                        break;
                    }
                }
                let span = start.to(self.prev_span);
                Ok(self.mk(ExprKind::List(elems), span))
            }
            Tok::P(Punct::LBrace) => {
                let block = self.block()?;
                let span = block.span;
                Ok(self.mk(ExprKind::Block(block), span))
            }
            Tok::Kw(Kw::If) => self.if_expr(),
            Tok::Kw(Kw::Return) => {
                self.bump();
                let value = if matches!(
                    self.tok(),
                    Tok::Newline | Tok::Eof | Tok::P(Punct::RBrace | Punct::RParen | Punct::Comma)
                ) {
                    None
                } else {
                    Some(Box::new(self.expr()?))
                };
                let span = start.to(self.prev_span);
                Ok(self.mk(ExprKind::Return(value), span))
            }
            Tok::Kw(Kw::Abort) => {
                self.bump();
                let value = self.expr()?;
                let span = start.to(value.span);
                Ok(self.mk(ExprKind::Abort(Box::new(value)), span))
            }
            Tok::Kw(Kw::Handle) => self.handle_expr(),
            Tok::Kw(Kw::Fn) => Err(self.unsupported("lambdas are", "Phase 0 (P0-10)")),
            Tok::Kw(Kw::Resume) => Err(self.unsupported("`resume` is", "Phase 7")),
            Tok::Kw(kw @ (Kw::Match | Kw::For | Kw::While | Kw::Loop | Kw::Break | Kw::Continue)) => {
                Err(self.unsupported(&format!("`{}` expressions are", kw.as_str()), "Phase 0 (P0-10)"))
            }
            Tok::P(Punct::DotDotDot) => Err(self.fail(
                "E0015",
                "`...` placeholders are only allowed in documentation examples",
                "replace with real code",
            )),
            _ => Err(self.expected("expression")),
        }
    }

    fn interpolation(&mut self, tokens: Vec<Token>, string_span: Span) -> PResult<Expr> {
        if tokens.iter().all(|t| matches!(t.tok, Tok::Newline | Tok::Eof)) {
            self.diags.push(
                Diagnostic::error("E0014", "empty interpolation in string")
                    .primary(string_span, "write `{{` for a literal brace"),
            );
            return Err(Fail);
        }
        let mut sub = Parser {
            toks: tokens,
            pos: 0,
            file: self.file,
            next_id: self.next_id,
            prev_span: string_span,
            diags: Vec::new(),
        };
        sub.skip_newlines();
        let result = sub.expr();
        sub.skip_newlines();
        if result.is_ok() && *sub.tok() != Tok::Eof {
            let _ = sub.fail("E0014", "unexpected tokens in string interpolation", "expected `}`");
        }
        self.next_id = sub.next_id;
        let failed = !sub.diags.is_empty();
        self.diags.append(&mut sub.diags);
        match result {
            Ok(e) if !failed => Ok(e),
            _ => Err(Fail),
        }
    }

    fn if_expr(&mut self) -> PResult<Expr> {
        let start = self.bump().span; // if
        let cond = self.expr()?;
        if *self.peek_past_newlines() == Tok::P(Punct::LBrace) {
            self.skip_newlines();
        }
        let then_block = self.block()?;
        let mut else_branch = None;
        if *self.peek_past_newlines() == Tok::Kw(Kw::Else) {
            self.skip_newlines();
            self.bump(); // else
            if self.at_kw(Kw::If) {
                else_branch = Some(Box::new(self.if_expr()?));
            } else {
                let block = self.block()?;
                let span = block.span;
                else_branch = Some(Box::new(self.mk(ExprKind::Block(block), span)));
            }
        }
        let span = start.to(self.prev_span);
        Ok(self.mk(ExprKind::If { cond: Box::new(cond), then_block, else_branch }, span))
    }

    fn handle_expr(&mut self) -> PResult<Expr> {
        let start = self.bump().span; // handle
        let mut bindings = Vec::new();
        loop {
            let effect = self.effect_path()?;
            if !self.at_kw(Kw::With) {
                return Err(self.expected("`with` after the effect in `handle`"));
            }
            self.bump();
            self.skip_newlines();
            if !self.at(Punct::LBrace) {
                return Err(self.unsupported(
                    "named handler values are",
                    "Phase 1; use an inline handler `with { fn op(..) { .. } }`",
                ));
            }
            self.bump(); // {
            let mut ops = Vec::new();
            loop {
                self.skip_newlines();
                if self.eat(Punct::RBrace) {
                    break;
                }
                if !self.at_kw(Kw::Fn) {
                    return Err(self.expected("`fn` in inline handler"));
                }
                ops.push(self.inline_op()?);
            }
            let span = effect.span.to(self.prev_span);
            bindings.push(HandlerBinding { effect, ops, span });
            if self.eat(Punct::Comma) {
                self.skip_newlines();
                continue;
            }
            break;
        }
        if *self.peek_past_newlines() == Tok::P(Punct::LBrace) {
            self.skip_newlines();
        }
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(self.mk(ExprKind::Handle { bindings, body }, span))
    }

    fn inline_op(&mut self) -> PResult<InlineOp> {
        let start = self.bump().span; // fn
        let name = self.ident("operation name")?;
        let params = self.param_list(false)?;
        let ret = if self.eat(Punct::Arrow) { Some(self.type_expr()?) } else { None };
        if *self.peek_past_newlines() == Tok::P(Punct::LBrace) {
            self.skip_newlines();
        }
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(InlineOp { name, params, ret, body, span })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> File {
        let (file, diags) = parse_file(FileId(0), src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:#?}");
        file
    }

    fn codes(src: &str) -> Vec<&'static str> {
        parse_file(FileId(0), src).1.iter().map(|d| d.code).collect()
    }

    #[test]
    fn parses_fn_with_uses_on_next_line() {
        let f = parse_ok("pub fn main()\n    uses [console]\n{\n    console.print(\"hi\")\n}\n");
        let Item::Fn(main) = &f.items[0] else { panic!() };
        assert!(main.is_pub);
        assert_eq!(main.uses.as_ref().unwrap().paths[0].dotted(), "console");
        assert_eq!(main.body.stmts.len(), 1);
    }

    #[test]
    fn parses_effect_decl_with_nesting() {
        let f = parse_ok("pub effect fs {\n    effect read {\n        fn read(path: Path) -> Bytes?\n    }\n}\n");
        let Item::Effect(fs) = &f.items[0] else { panic!() };
        let EffectMember::Effect(read) = &fs.members[0] else { panic!() };
        assert_eq!(read.name.name, "read");
    }

    #[test]
    fn precedence_and_continuation() {
        let f = parse_ok("fn f() -> Int {\n    1 +\n        2 * 3\n}\n");
        let Item::Fn(func) = &f.items[0] else { panic!() };
        let Stmt::Expr(e) = &func.body.stmts[0] else { panic!() };
        let ExprKind::Binary(BinOp::Add, _, rhs) = &e.kind else { panic!("{e:?}") };
        assert!(matches!(rhs.kind, ExprKind::Binary(BinOp::Mul, _, _)));
    }

    #[test]
    fn parses_test_with_inline_handler() {
        let src = r#"
test "captures" {
    var out = ""
    handle console with {
        fn print(text) { out = out + text }
    } {
        console.print("x")
    }
}
"#;
        let f = parse_ok(src);
        let Item::Test(t) = &f.items[0] else { panic!() };
        assert_eq!(t.name, "captures");
    }

    #[test]
    fn named_args_and_interpolation() {
        parse_ok("fn f() {\n    g(1, name: \"a {x + 1}\")\n}\n");
    }

    #[test]
    fn reports_unsupported_items_and_recovers() {
        assert_eq!(codes("type User {\n    id: Int\n}\n\nfn ok() {}\n"), vec!["E0009"]);
    }

    #[test]
    fn statement_separator_error() {
        assert_eq!(codes("fn f() {\n    a b\n}\n"), vec!["E0012"]);
    }

    #[test]
    fn missing_param_type() {
        assert_eq!(codes("fn f(x) {}\n"), vec!["E0010"]);
    }
}
