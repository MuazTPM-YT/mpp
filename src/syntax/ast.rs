use super::Span;
use std::rc::Rc;

pub type Name = Rc<str>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IntDiv,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    Approx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum FPart {
    Lit(String),
    Expr(Box<Expr>, Option<String>),
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<Name>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    FStr(Vec<FPart>),
    Name(Name),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Arg>),
    Prop(Box<Expr>, Name),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    Range(Box<Expr>, Box<Expr>, bool),
    Lambda(Rc<FnDecl>),
    Super(Name),
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Name,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: Name,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Import {
    Builtin(Name),
    File(Rc<str>),
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Expr(Expr),
    // one target = plain, many targets = unpack a list
    Assign(Vec<Expr>, Expr),
    AugAssign(BinOp, Expr, Expr),
    Const(Name, Expr),
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>),
    While(Expr, Vec<Stmt>),
    For(Vec<(Name, Span)>, Expr, Vec<Stmt>),
    Break,
    Continue,
    Return(Option<Expr>),
    Fn(Rc<FnDecl>),
    Class { name: Name, sup: Option<Expr>, methods: Vec<Rc<FnDecl>> },
    Import(Vec<(Import, Option<Name>, Span)>),
    Try(Vec<Stmt>, Option<(Name, Span)>, Vec<Stmt>),
    Throw(Expr),
    Global(Vec<Name>),
    Nonlocal(Vec<Name>),
    // test / experiment / bench / property block, top level only
    TestBlock { kind: TestKind, name: Rc<str>, gens: Vec<(Name, Expr)>, opts: Vec<(Name, Expr)>, body: Vec<Stmt> },
    // expect cond [within tol]
    Expect(Expr, Option<Expr>),
    // report [label:] value
    Report(Option<Expr>, Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    Test,
    Experiment,
    Bench,
    Property,
}

impl TestKind {
    pub fn from_word(w: &str) -> Option<TestKind> {
        Some(match w {
            "test" => TestKind::Test,
            "experiment" => TestKind::Experiment,
            "bench" => TestKind::Bench,
            "property" => TestKind::Property,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        match self {
            TestKind::Test => "test",
            TestKind::Experiment => "experiment",
            TestKind::Bench => "bench",
            TestKind::Property => "property",
        }
    }
}
