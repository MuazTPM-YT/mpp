use crate::syntax::{Diag, Span};
use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term::{
    self, Config,
    termcolor::{ColorChoice, StandardStream},
};
use std::collections::HashMap;
use std::io::IsTerminal;

// every source file we loaded, for pretty errors
#[derive(Default)]
pub struct Sources {
    files: SimpleFiles<String, String>,
    ids: HashMap<String, usize>,
}

impl Sources {
    pub fn add(&mut self, name: &str, src: String) -> usize {
        let id = self.files.add(name.to_string(), src);
        self.ids.insert(name.to_string(), id);
        id
    }

    pub fn source(&self, name: &str) -> Option<&str> {
        let id = *self.ids.get(name)?;
        self.files.get(id).ok().map(|f| f.source().as_str())
    }

    fn build(&self, name: &str, msg: &str, span: Option<Span>, label: &str, notes: Vec<String>) -> Option<(usize, Diagnostic<usize>)> {
        let id = *self.ids.get(name)?;
        let mut d = Diagnostic::error().with_message(msg).with_notes(notes);
        if let Some(s) = span {
            let len = self.files.get(id).ok()?.source().len();
            let start = (s.start as usize).min(len);
            let end = (s.end as usize).clamp(start, len);
            d = d.with_labels(vec![Label::primary(id, start..end).with_message(label)]);
        }
        Some((id, d))
    }

    // compile error to stderr
    pub fn report(&self, file: &str, diag: &Diag) {
        let notes = diag.note.iter().cloned().collect();
        match self.build(file, &diag.msg, Some(diag.span), "", notes) {
            Some((_, d)) => self.emit(&d),
            None => eprintln!("error: {} ({file})", diag.msg),
        }
    }

    // compile error as plain text (tests, LSP)
    pub fn render(&self, file: &str, diag: &Diag) -> String {
        let notes = diag.note.iter().cloned().collect();
        match self.build(file, &diag.msg, Some(diag.span), "", notes) {
            Some((_, d)) => term::emit_into_string(&Config::default(), &self.files, &d).unwrap_or_default(),
            None => format!("error: {}\n", diag.msg),
        }
    }

    // runtime error with snippet at line:col (1-based)
    pub fn report_at(&self, file: &str, line: u32, col: u32, msg: &str, notes: Vec<String>) -> bool {
        let Some(src) = self.source(file) else {
            return false;
        };
        let Some(off) = offset_of(src, line, col) else {
            return false;
        };
        let end = src[off..].find(['\n', ' ', '(', ')', ',']).map_or(src.len(), |n| off + n.max(1));
        match self.build(file, msg, Some(Span::new(off, end)), "here", notes) {
            Some((_, d)) => {
                self.emit(&d);
                true
            }
            None => false,
        }
    }

    fn emit(&self, d: &Diagnostic<usize>) {
        let color = if std::io::stderr().is_terminal() { ColorChoice::Auto } else { ColorChoice::Never };
        let w = StandardStream::stderr(color);
        let _ = term::emit_to_write_style(&mut w.lock(), &Config::default(), &self.files, d);
    }
}

// line/col (1-based) to byte offset
fn offset_of(src: &str, line: u32, col: u32) -> Option<usize> {
    let start = if line <= 1 { 0 } else { src.match_indices('\n').nth(line as usize - 2)?.0 + 1 };
    let line_text = &src[start..];
    let col_off: usize = line_text.chars().take(col.saturating_sub(1) as usize).map(char::len_utf8).sum();
    Some(start + col_off)
}

// byte offset to 1-based line/col, fast via line-start table
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(src: &str) -> LineIndex {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { starts }
    }

    pub fn line_col(&self, src: &str, off: u32) -> (u32, u32) {
        let off = (off as usize).min(src.len());
        let line = self.starts.partition_point(|&s| s <= off) - 1;
        let start = self.starts[line];
        let col = src.get(start..off).map_or(off - start, |s| s.chars().count()) + 1;
        (line as u32 + 1, col as u32)
    }
}
