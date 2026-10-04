//! Hand-written lexer. Newlines are significant tokens; comments and other
//! whitespace are dropped (the lossless rowan CST arrives with P0-05/P0-07).

use nova_diag::{Diagnostic, FileId, Span};

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(Vec<StrPiece>),
    Kw(Kw),
    P(Punct),
    Newline,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPiece {
    Text(String),
    /// Tokens of an interpolated `{expr}`, terminated by `Eof`.
    Interp(Vec<Token>),
}

macro_rules! keywords {
    ($($variant:ident = $text:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Kw { $($variant,)* }

        impl Kw {
            pub fn from_keyword(s: &str) -> Option<Kw> {
                match s { $($text => Some(Kw::$variant),)* _ => None }
            }

            pub fn as_str(self) -> &'static str {
                match self { $(Kw::$variant => $text,)* }
            }
        }
    };
}

keywords! {
    Abort = "abort",
    Break = "break",
    Continue = "continue",
    Effect = "effect",
    Else = "else",
    Enum = "enum",
    Eval = "eval",
    False = "false",
    Fn = "fn",
    For = "for",
    Handle = "handle",
    Handler = "handler",
    If = "if",
    Impl = "impl",
    In = "in",
    Let = "let",
    Loop = "loop",
    Match = "match",
    Model = "model",
    Mut = "mut",
    Pub = "pub",
    Resume = "resume",
    Return = "return",
    Test = "test",
    Tool = "tool",
    Trait = "trait",
    True = "true",
    Type = "type",
    Use = "use",
    Uses = "uses",
    Var = "var",
    While = "while",
    With = "with",
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Punct {
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    DotDotDot,
    Arrow,
    FatArrow,
    Eq,
    EqEq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    AndAnd,
    OrOr,
    Question,
    PlusEq,
    MinusEq,
    At,
    Pipe,
}

impl Punct {
    pub fn as_str(self) -> &'static str {
        use Punct::*;
        match self {
            LParen => "(",
            RParen => ")",
            LBrace => "{",
            RBrace => "}",
            LBracket => "[",
            RBracket => "]",
            Comma => ",",
            Colon => ":",
            Dot => ".",
            DotDotDot => "...",
            Arrow => "->",
            FatArrow => "=>",
            Eq => "=",
            EqEq => "==",
            Ne => "!=",
            Lt => "<",
            Gt => ">",
            Le => "<=",
            Ge => ">=",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Bang => "!",
            AndAnd => "&&",
            OrOr => "||",
            Question => "?",
            PlusEq => "+=",
            MinusEq => "-=",
            At => "@",
            Pipe => "|",
        }
    }
}

impl Tok {
    /// Human-readable description for "expected X, found Y" messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(name) => format!("identifier `{name}`"),
            Tok::Int(_) | Tok::Float(_) => "number".into(),
            Tok::Str(_) => "string".into(),
            Tok::Kw(kw) => format!("keyword `{}`", kw.as_str()),
            Tok::P(p) => format!("`{}`", p.as_str()),
            Tok::Newline => "newline".into(),
            Tok::Eof => "end of file".into(),
        }
    }
}

/// Lexes `src`. Spans are offset by `base` so interpolated sub-lexes keep file positions.
pub fn lex(file: FileId, src: &str, base: usize, diags: &mut Vec<Diagnostic>) -> Vec<Token> {
    let mut lx = Lexer { file, src, pos: 0, base, diags, tokens: Vec::new() };
    lx.run();
    lx.tokens
}

struct Lexer<'a, 'd> {
    file: FileId,
    src: &'a str,
    pos: usize,
    base: usize,
    diags: &'d mut Vec<Diagnostic>,
    tokens: Vec<Token>,
}

impl Lexer<'_, '_> {
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.file, self.base + start, self.base + end)
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        let span = self.span(start, self.pos);
        self.tokens.push(Token { tok, span });
    }

    fn run(&mut self) {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                '\n' => {
                    self.pos += 1;
                    self.push(Tok::Newline, start);
                }
                c if c.is_whitespace() => self.pos += c.len_utf8(),
                '/' if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.pos += c.len_utf8();
                    }
                }
                '"' => self.string(),
                c if c.is_ascii_digit() => self.number(),
                c if c == '_' || c.is_alphabetic() => {
                    while let Some(c) = self.peek() {
                        if c == '_' || c.is_alphanumeric() {
                            self.pos += c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    let text = &self.src[start..self.pos];
                    let tok = match Kw::from_keyword(text) {
                        Some(kw) => Tok::Kw(kw),
                        None => Tok::Ident(text.to_string()),
                    };
                    self.push(tok, start);
                }
                _ => self.punct(c),
            }
        }
        let end = self.src.len();
        self.tokens.push(Token { tok: Tok::Eof, span: self.span(end, end) });
    }

    fn punct(&mut self, c: char) {
        use Punct::*;
        let start = self.pos;
        let two = self.src[self.pos..].get(..2).unwrap_or("");
        let three = self.src[self.pos..].get(..3).unwrap_or("");
        let (p, len) = if three == "..." {
            (DotDotDot, 3)
        } else {
            match two {
                "->" => (Arrow, 2),
                "=>" => (FatArrow, 2),
                "==" => (EqEq, 2),
                "!=" => (Ne, 2),
                "<=" => (Le, 2),
                ">=" => (Ge, 2),
                "&&" => (AndAnd, 2),
                "||" => (OrOr, 2),
                "+=" => (PlusEq, 2),
                "-=" => (MinusEq, 2),
                _ => {
                    let p = match c {
                        '(' => LParen,
                        ')' => RParen,
                        '{' => LBrace,
                        '}' => RBrace,
                        '[' => LBracket,
                        ']' => RBracket,
                        ',' => Comma,
                        ':' => Colon,
                        '.' => Dot,
                        '=' => Eq,
                        '<' => Lt,
                        '>' => Gt,
                        '+' => Plus,
                        '-' => Minus,
                        '*' => Star,
                        '/' => Slash,
                        '%' => Percent,
                        '!' => Bang,
                        '?' => Question,
                        '@' => At,
                        '|' => Pipe,
                        _ => {
                            self.pos += c.len_utf8();
                            let span = self.span(start, self.pos);
                            self.diags.push(
                                Diagnostic::error("E0001", format!("unexpected character `{c}`"))
                                    .primary(span, "not valid here"),
                            );
                            return;
                        }
                    };
                    (p, 1)
                }
            }
        };
        self.pos += len;
        self.push(Tok::P(p), start);
    }

    fn number(&mut self) {
        let start = self.pos;
        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                self.pos += 1;
            } else if c == '.' && !is_float && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) {
                is_float = true;
                self.pos += 1;
            } else {
                break;
            }
        }
        let text: String = self.src[start..self.pos].chars().filter(|&c| c != '_').collect();
        let tok = if is_float {
            Tok::Float(text.parse().unwrap_or(0.0))
        } else {
            match text.parse::<i64>() {
                Ok(n) => Tok::Int(n),
                Err(_) => {
                    let span = self.span(start, self.pos);
                    self.diags.push(
                        Diagnostic::error("E0007", "integer literal is too large")
                            .primary(span, "does not fit in a 64-bit `Int`"),
                    );
                    Tok::Int(0)
                }
            }
        };
        self.push(tok, start);
    }

    fn string(&mut self) {
        let start = self.pos;
        if self.src[self.pos..].starts_with("\"\"\"") {
            self.pos += 3;
            let close = self.src[self.pos..].find("\"\"\"").map(|i| self.pos + i + 3);
            self.pos = close.unwrap_or(self.src.len());
            let span = self.span(start, self.pos);
            self.diags.push(
                Diagnostic::error("E0004", "triple-quoted strings are not supported yet")
                    .primary(span, "planned for P0-06"),
            );
            self.push(Tok::Str(Vec::new()), start);
            return;
        }
        self.pos += 1;
        let mut pieces = Vec::new();
        let mut text = String::new();
        loop {
            let Some(c) = self.peek() else {
                self.unterminated(start);
                break;
            };
            match c {
                '"' => {
                    self.pos += 1;
                    break;
                }
                '\n' => {
                    self.unterminated(start);
                    break;
                }
                '\\' => {
                    let esc_start = self.pos;
                    self.pos += 1;
                    let e = self.peek();
                    self.pos += e.map_or(0, char::len_utf8);
                    match e {
                        Some('n') => text.push('\n'),
                        Some('t') => text.push('\t'),
                        Some('r') => text.push('\r'),
                        Some('0') => text.push('\0'),
                        Some('\\') => text.push('\\'),
                        Some('"') => text.push('"'),
                        Some('\'') => text.push('\''),
                        _ => {
                            let span = self.span(esc_start, self.pos);
                            self.diags.push(
                                Diagnostic::error("E0002", "unknown escape sequence")
                                    .primary(span, "use `\\\\` for a literal backslash"),
                            );
                        }
                    }
                }
                '{' if self.peek_at(1) == Some('{') => {
                    self.pos += 2;
                    text.push('{');
                }
                '}' if self.peek_at(1) == Some('}') => {
                    self.pos += 2;
                    text.push('}');
                }
                '{' => {
                    if !text.is_empty() {
                        pieces.push(StrPiece::Text(std::mem::take(&mut text)));
                    }
                    self.pos += 1;
                    let inner_start = self.pos;
                    let inner_end = self.interpolation_end();
                    let inner = &self.src[inner_start..inner_end];
                    let tokens = lex(self.file, inner, self.base + inner_start, self.diags);
                    pieces.push(StrPiece::Interp(tokens));
                    self.pos = (inner_end + 1).min(self.src.len());
                }
                '}' => {
                    let span = self.span(self.pos, self.pos + 1);
                    self.pos += 1;
                    self.diags.push(
                        Diagnostic::error("E0005", "unmatched `}` in string")
                            .primary(span, "write `}}` for a literal brace"),
                    );
                }
                c => {
                    self.pos += c.len_utf8();
                    text.push(c);
                }
            }
        }
        if !text.is_empty() || pieces.is_empty() {
            pieces.push(StrPiece::Text(text));
        }
        self.push(Tok::Str(pieces), start);
    }

    /// Finds the `}` closing an interpolation that starts at `self.pos`,
    /// skipping nested braces and nested string literals.
    fn interpolation_end(&mut self) -> usize {
        let bytes = self.src.as_bytes();
        let mut depth = 1;
        let mut i = self.pos;
        let mut in_str = false;
        while i < bytes.len() {
            let b = bytes[i];
            if in_str {
                match b {
                    b'\\' => i += 1,
                    b'"' => in_str = false,
                    _ => {}
                }
            } else {
                match b {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return i;
                        }
                    }
                    b'\n' => break,
                    _ => {}
                }
            }
            i += 1;
        }
        let span = self.span(self.pos - 1, i);
        self.diags
            .push(Diagnostic::error("E0003", "unterminated interpolation in string").primary(span, "missing `}`"));
        i
    }

    fn unterminated(&mut self, start: usize) {
        let span = self.span(start, self.pos);
        self.diags
            .push(Diagnostic::error("E0003", "unterminated string literal").primary(span, "missing closing `\"`"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let mut diags = Vec::new();
        let t = lex(FileId(0), src, 0, &mut diags);
        assert!(diags.is_empty(), "{diags:?}");
        t.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn keywords_idents_and_punct() {
        assert_eq!(
            toks("pub fn main() -> x"),
            vec![
                Tok::Kw(Kw::Pub),
                Tok::Kw(Kw::Fn),
                Tok::Ident("main".into()),
                Tok::P(Punct::LParen),
                Tok::P(Punct::RParen),
                Tok::P(Punct::Arrow),
                Tok::Ident("x".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn numbers_and_member_access_on_ints() {
        assert_eq!(toks("1_000 2.5"), vec![Tok::Int(1000), Tok::Float(2.5), Tok::Eof]);
        assert_eq!(toks("2.seconds"), vec![Tok::Int(2), Tok::P(Punct::Dot), Tok::Ident("seconds".into()), Tok::Eof]);
    }

    #[test]
    fn comments_dropped_newlines_kept() {
        assert_eq!(toks("a // hi\nb"), vec![Tok::Ident("a".into()), Tok::Newline, Tok::Ident("b".into()), Tok::Eof]);
    }

    #[test]
    fn string_interpolation_with_nested_string() {
        let t = toks(r#""a {f("}}")} b {{x}}""#);
        let Tok::Str(pieces) = &t[0] else { panic!() };
        assert_eq!(pieces.len(), 3);
        assert_eq!(pieces[0], StrPiece::Text("a ".into()));
        let StrPiece::Interp(inner) = &pieces[1] else { panic!() };
        assert_eq!(inner[0].tok, Tok::Ident("f".into()));
        assert_eq!(pieces[2], StrPiece::Text(" b {x}".into()));
    }

    #[test]
    fn interpolation_spans_point_into_file() {
        let mut diags = Vec::new();
        let t = lex(FileId(0), r#"x "hi {name}""#, 0, &mut diags);
        let Tok::Str(pieces) = &t[1].tok else { panic!() };
        let StrPiece::Interp(inner) = &pieces[1] else { panic!() };
        assert_eq!((inner[0].span.start, inner[0].span.end), (7, 11));
    }

    #[test]
    fn unterminated_string_reports_e0003() {
        let mut diags = Vec::new();
        lex(FileId(0), "\"abc\nx", 0, &mut diags);
        assert_eq!(diags[0].code, "E0003");
    }
}
