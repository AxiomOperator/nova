//! Source files, spans, and diagnostics shared by every Nova compiler stage.

use std::fmt::Write;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

/// A byte range within one source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: usize, end: usize) -> Span {
        Span { file, start: start as u32, end: end as u32 }
    }

    /// The smallest span covering both `self` and `other` (same file).
    pub fn to(self, other: Span) -> Span {
        Span { file: self.file, start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

pub struct SourceFile {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        let text = text.into();
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        SourceFile { name: name.into(), text, line_starts }
    }

    /// 1-based line and 1-based column (in characters) of a byte offset.
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let offset = (offset as usize).min(self.text.len());
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let col = self.text[self.line_starts[line]..offset].chars().count() + 1;
        (line + 1, col)
    }

    /// Text of a 1-based line, without its line terminator.
    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line - 1];
        let end = self.line_starts.get(line).copied().unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        self.files.push(SourceFile::new(name, text));
        FileId(self.files.len() as u32 - 1)
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }

    /// `name:line` for a span, used in provenance notes.
    pub fn location(&self, span: Span) -> String {
        let file = self.get(span.file);
        let (line, _) = file.line_col(span.start);
        format!("{}:{}", file.name, line)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
    pub primary: bool,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Registered code, e.g. `"E0402"`. See `docs/spec/errors.md`.
    pub code: &'static str,
    pub message: String,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub help: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(Severity::Error, code, message)
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(Severity::Warning, code, message)
    }

    fn new(severity: Severity, code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic { severity, code, message: message.into(), labels: Vec::new(), notes: Vec::new(), help: Vec::new() }
    }

    pub fn primary(mut self, span: Span, message: impl Into<String>) -> Diagnostic {
        self.labels.push(Label { span, message: message.into(), primary: true });
        self
    }

    pub fn secondary(mut self, span: Span, message: impl Into<String>) -> Diagnostic {
        self.labels.push(Label { span, message: message.into(), primary: false });
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help.push(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn primary_span(&self) -> Option<Span> {
        self.labels.iter().find(|l| l.primary).or(self.labels.first()).map(|l| l.span)
    }
}

/// Renders a diagnostic as plain text in a rustc-like layout. Deterministic, no color.
pub fn render(diag: &Diagnostic, sources: &SourceMap) -> String {
    let mut out = String::new();
    let level = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let _ = writeln!(out, "{level}[{}]: {}", diag.code, diag.message);

    let Some(primary) = diag.primary_span() else {
        for note in &diag.notes {
            let _ = writeln!(out, "  = note: {note}");
        }
        for help in &diag.help {
            let _ = writeln!(out, "  = help: {help}");
        }
        return out;
    };
    let file = sources.get(primary.file);

    // Labels in the primary file, ordered by position.
    let mut labels: Vec<(usize, usize, usize, &Label)> = diag
        .labels
        .iter()
        .filter(|l| l.span.file == primary.file)
        .map(|l| {
            let (line, col) = file.line_col(l.span.start);
            let (end_line, end_col) = file.line_col(l.span.end);
            let width = if end_line == line {
                end_col.saturating_sub(col).max(1)
            } else {
                file.line_text(line).chars().count().saturating_sub(col - 1).max(1)
            };
            (line, col, width, l)
        })
        .collect();
    labels.sort_by_key(|&(line, col, _, l)| (line, col, !l.primary));

    let max_line = labels.iter().map(|l| l.0).max().unwrap_or(1);
    let gutter = max_line.to_string().len();
    let pad = " ".repeat(gutter);
    let (pline, pcol) = file.line_col(primary.start);
    let _ = writeln!(out, "{pad}--> {}:{pline}:{pcol}", file.name);
    let _ = writeln!(out, "{pad} |");

    let mut prev_line: Option<usize> = None;
    for &(line, col, width, label) in &labels {
        if prev_line != Some(line) {
            if prev_line.is_some_and(|prev| line > prev + 1) {
                let _ = writeln!(out, "{pad}...");
            }
            let _ = writeln!(out, "{line:>gutter$} | {}", file.line_text(line));
            prev_line = Some(line);
        }
        let marker = if label.primary { "^" } else { "-" };
        let underline = format!("{}{}", " ".repeat(col - 1), marker.repeat(width));
        if label.message.is_empty() {
            let _ = writeln!(out, "{pad} | {underline}");
        } else {
            let _ = writeln!(out, "{pad} | {underline} {}", label.message);
        }
    }

    for label in diag.labels.iter().filter(|l| l.span.file != primary.file) {
        let _ = writeln!(out, "{pad} = note: {}: {}", sources.location(label.span), label.message);
    }
    if !diag.notes.is_empty() || !diag.help.is_empty() {
        let _ = writeln!(out, "{pad} |");
    }
    for note in &diag.notes {
        let _ = writeln!(out, "{pad} = note: {note}");
    }
    for help in &diag.help {
        let _ = writeln!(out, "{pad} = help: {help}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_handles_multibyte_and_crlf() {
        let f = SourceFile::new("t", "aé\r\nb");
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(3), (1, 3));
        assert_eq!(f.line_col(5), (2, 1));
        assert_eq!(f.line_text(1), "aé");
    }

    #[test]
    fn renders_primary_and_secondary_labels() {
        let mut map = SourceMap::default();
        let id = map.add("x.nova", "pub fn main() {\n    helper()\n}\n");
        let d = Diagnostic::error("E0402", "`main` uses `console` but declares `uses []`")
            .secondary(Span::new(id, 7, 11), "declared here")
            .primary(Span::new(id, 20, 28), "`console` enters here")
            .note("helper (x.nova:1) → console.print");
        let text = render(&d, &map);
        let expected = "\
error[E0402]: `main` uses `console` but declares `uses []`
 --> x.nova:2:5
  |
1 | pub fn main() {
  |        ---- declared here
2 |     helper()
  |     ^^^^^^^^ `console` enters here
  |
  = note: helper (x.nova:1) → console.print
";
        assert_eq!(text, expected);
    }
}
