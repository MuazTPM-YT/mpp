use super::Span;
use std::rc::Rc;

// piece of an f-string: plain text or {expr:spec}
#[derive(Debug, Clone, PartialEq)]
pub enum FPiece {
    Lit(String),
    Expr { src: String, offset: u32, spec: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(Rc<str>),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    FStr(Vec<FPiece>),

    Fn,
    Return,
    If,
    Elif,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Class,
    Import,
    As,
    Const,
    Try,
    Catch,
    Throw,
    Nil,
    True,
    False,
    And,
    Or,
    Not,
    Global,
    Nonlocal,
    Super,

    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Dot,
    DotDot,
    DotDotEq,
    Colon,
    Semi,
    Question,
    Arrow,
    Plus,
    Minus,
    Star,
    StarStar,
    Slash,
    SlashSlash,
    Percent,
    Assign,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    Approx,

    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

// keyword table
pub fn keyword(s: &str) -> Option<Tok> {
    Some(match s {
        "fn" => Tok::Fn,
        "return" => Tok::Return,
        "if" => Tok::If,
        "elif" => Tok::Elif,
        "else" => Tok::Else,
        "while" => Tok::While,
        "for" => Tok::For,
        "in" => Tok::In,
        "break" => Tok::Break,
        "continue" => Tok::Continue,
        "class" => Tok::Class,
        "import" => Tok::Import,
        "as" => Tok::As,
        "const" => Tok::Const,
        "try" => Tok::Try,
        "catch" => Tok::Catch,
        "throw" => Tok::Throw,
        "nil" => Tok::Nil,
        "true" => Tok::True,
        "false" => Tok::False,
        "and" => Tok::And,
        "or" => Tok::Or,
        "not" => Tok::Not,
        "global" => Tok::Global,
        "nonlocal" => Tok::Nonlocal,
        "super" => Tok::Super,
        _ => return None,
    })
}

// human name for error messages
pub fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("name `{s}`"),
        Tok::Int(n) => format!("number `{n}`"),
        Tok::Float(n) => format!("number `{n}`"),
        Tok::Str(_) | Tok::FStr(_) => "string".into(),
        Tok::Newline => "end of line".into(),
        Tok::Eof => "end of file".into(),
        other => format!("`{}`", symbol(other)),
    }
}

// keyword spelled as a word (usable as field or argument name)
pub fn keyword_word(t: &Tok) -> Option<&'static str> {
    let s = symbol(t);
    (s != "?" && s.chars().all(|c| c.is_ascii_alphabetic())).then_some(s)
}

fn symbol(t: &Tok) -> &'static str {
    match t {
        Tok::Fn => "fn",
        Tok::Return => "return",
        Tok::If => "if",
        Tok::Elif => "elif",
        Tok::Else => "else",
        Tok::While => "while",
        Tok::For => "for",
        Tok::In => "in",
        Tok::Break => "break",
        Tok::Continue => "continue",
        Tok::Class => "class",
        Tok::Import => "import",
        Tok::As => "as",
        Tok::Const => "const",
        Tok::Try => "try",
        Tok::Catch => "catch",
        Tok::Throw => "throw",
        Tok::Nil => "nil",
        Tok::True => "true",
        Tok::False => "false",
        Tok::And => "and",
        Tok::Or => "or",
        Tok::Not => "not",
        Tok::Global => "global",
        Tok::Nonlocal => "nonlocal",
        Tok::Super => "super",
        Tok::LParen => "(",
        Tok::RParen => ")",
        Tok::LBracket => "[",
        Tok::RBracket => "]",
        Tok::LBrace => "{",
        Tok::RBrace => "}",
        Tok::Comma => ",",
        Tok::Dot => ".",
        Tok::DotDot => "..",
        Tok::DotDotEq => "..=",
        Tok::Colon => ":",
        Tok::Semi => ";",
        Tok::Question => "?",
        Tok::Arrow => "=>",
        Tok::Plus => "+",
        Tok::Minus => "-",
        Tok::Star => "*",
        Tok::StarStar => "**",
        Tok::Slash => "/",
        Tok::SlashSlash => "//",
        Tok::Percent => "%",
        Tok::Assign => "=",
        Tok::EqEq => "==",
        Tok::Ne => "!=",
        Tok::Lt => "<",
        Tok::Le => "<=",
        Tok::Gt => ">",
        Tok::Ge => ">=",
        Tok::PlusEq => "+=",
        Tok::MinusEq => "-=",
        Tok::StarEq => "*=",
        Tok::SlashEq => "/=",
        Tok::PercentEq => "%=",
        Tok::Approx => "~=",
        _ => "?",
    }
}
