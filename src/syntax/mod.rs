pub mod ast;
pub mod lexer;
pub mod parser;
pub mod token;

use serde::{Deserialize, Serialize};

// byte range in one source file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start: start as u32, end: end as u32 }
    }

    // smallest span covering both
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

// one compile problem, pinned to a span
#[derive(Debug, Clone)]
pub struct Diag {
    pub msg: String,
    pub span: Span,
    pub note: Option<String>,
}

impl Diag {
    pub fn new(msg: impl Into<String>, span: Span) -> Diag {
        Diag { msg: msg.into(), span, note: None }
    }

    pub fn note(mut self, note: impl Into<String>) -> Diag {
        self.note = Some(note.into());
        self
    }
}

// lex + parse a whole file
pub fn parse(src: &str) -> Result<Vec<ast::Stmt>, Vec<Diag>> {
    let toks = lexer::lex(src, 0).map_err(|d| vec![d])?;
    parser::Parser::new(toks).program()
}
