use super::ast::*;
use super::lexer::lex;
use super::token::{FPiece, Tok, Token, describe};
use super::{Diag, Span};
use std::rc::Rc;

type PResult<T> = Result<T, Diag>;

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diag>,
}

impl Parser {
    pub fn new(toks: Vec<Token>) -> Parser {
        Parser { toks, pos: 0, diags: Vec::new() }
    }

    // whole file; keeps going after errors to report many at once
    pub fn program(mut self) -> Result<Vec<Stmt>, Vec<Diag>> {
        let mut out = Vec::new();
        loop {
            self.skip_seps();
            if self.at(&Tok::Eof) {
                break;
            }
            match self.statement() {
                Ok(s) => out.push(s),
                Err(d) => {
                    self.diags.push(d);
                    self.recover();
                }
            }
        }
        if self.diags.is_empty() { Ok(out) } else { Err(self.diags) }
    }

    // parse one expression (for f-string parts and the REPL)
    pub fn lone_expr(mut self) -> PResult<Expr> {
        self.skip_newlines();
        let e = self.expr()?;
        self.skip_newlines();
        if !self.at(&Tok::Eof) {
            return Err(self.unexpected("end of expression"));
        }
        Ok(e)
    }

    // skip to next line at this nesting level
    fn recover(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace if depth == 0 => {
                    self.pos += 1;
                    return;
                }
                Tok::RBrace => depth -= 1,
                Tok::Newline | Tok::Semi if depth <= 0 => {
                    self.pos += 1;
                    return;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }

    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn unexpected(&self, want: &str) -> Diag {
        Diag::new(format!("expected {want}, found {}", describe(self.peek())), self.span())
    }

    fn expect(&mut self, t: Tok, want: &str) -> PResult<Span> {
        if self.at(&t) { Ok(self.bump().span) } else { Err(self.unexpected(want)) }
    }

    fn ident(&mut self, want: &str) -> PResult<(Name, Span)> {
        match self.peek().clone() {
            Tok::Ident(n) => Ok((n, self.bump().span)),
            _ => Err(self.unexpected(want)),
        }
    }

    fn skip_newlines(&mut self) {
        while self.at(&Tok::Newline) {
            self.bump();
        }
    }

    fn skip_seps(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.bump();
        }
    }

    // peek past newlines for a keyword like `else`; eat newlines only on hit
    fn next_line_is(&mut self, t: &Tok) -> bool {
        let mut i = self.pos;
        while self.toks[i].tok == Tok::Newline {
            i += 1;
        }
        if &self.toks[i].tok == t {
            self.pos = i;
            true
        } else {
            false
        }
    }

    fn end_stmt(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline | Tok::Semi => {
                self.bump();
                Ok(())
            }
            Tok::RBrace | Tok::Eof => Ok(()),
            _ => Err(self.unexpected("end of line")),
        }
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut out = Vec::new();
        loop {
            self.skip_seps();
            if self.eat(&Tok::RBrace) {
                return Ok(out);
            }
            if self.at(&Tok::Eof) {
                return Err(self.unexpected("`}`"));
            }
            match self.statement() {
                Ok(s) => out.push(s),
                Err(d) => {
                    self.diags.push(d);
                    self.recover();
                    if matches!(self.toks[self.pos - 1].tok, Tok::RBrace) {
                        return Ok(out);
                    }
                }
            }
        }
    }

    fn statement(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let kind = match self.peek() {
            Tok::Fn if matches!(self.peek_at(1), Tok::Ident(_)) => {
                self.bump();
                StmtKind::Fn(self.fn_decl(start)?)
            }
            Tok::Class => self.class()?,
            Tok::If => self.if_stmt()?,
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                StmtKind::While(cond, self.block()?)
            }
            Tok::For => self.for_stmt()?,
            Tok::Return => {
                self.bump();
                if matches!(self.peek(), Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof) {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.expr_list()?))
                }
            }
            Tok::Break => {
                self.bump();
                StmtKind::Break
            }
            Tok::Continue => {
                self.bump();
                StmtKind::Continue
            }
            Tok::Import => self.import()?,
            Tok::Const => {
                self.bump();
                let (name, _) = self.ident("constant name")?;
                self.expect(Tok::Assign, "`=`")?;
                StmtKind::Const(name, self.expr()?)
            }
            Tok::Try => self.try_stmt()?,
            Tok::Throw => {
                self.bump();
                StmtKind::Throw(self.expr()?)
            }
            Tok::Global | Tok::Nonlocal => {
                let global = self.bump().tok == Tok::Global;
                let mut names = vec![self.ident("name")?.0];
                while self.eat(&Tok::Comma) {
                    names.push(self.ident("name")?.0);
                }
                if global { StmtKind::Global(names) } else { StmtKind::Nonlocal(names) }
            }
            Tok::Ident(w) if TestKind::from_word(w).is_some() && matches!(self.peek_at(1), Tok::Str(_)) => self.test_block()?,
            Tok::Ident(w) if (&**w == "expect" || &**w == "report") && self.word_is_keyword() => {
                let is_expect = &**w == "expect";
                self.bump();
                let first = self.expr()?;
                if is_expect {
                    let within = match self.peek() {
                        Tok::Ident(n) if &**n == "within" => {
                            self.bump();
                            Some(self.expr()?)
                        }
                        _ => None,
                    };
                    StmtKind::Expect(first, within)
                } else if self.eat(&Tok::Colon) {
                    StmtKind::Report(Some(first), self.expr()?)
                } else {
                    StmtKind::Report(None, first)
                }
            }
            _ => self.simple()?,
        };
        let span = start.to(self.prev_span());
        self.end_stmt()?;
        Ok(Stmt { kind, span })
    }

    // `expect`/`report` used as a statement word, not a variable
    fn word_is_keyword(&self) -> bool {
        !matches!(
            self.peek_at(1),
            Tok::Assign
                | Tok::PlusEq
                | Tok::MinusEq
                | Tok::StarEq
                | Tok::SlashEq
                | Tok::PercentEq
                | Tok::Comma
                | Tok::Dot
                | Tok::Newline
                | Tok::Semi
                | Tok::Eof
                | Tok::RBrace
        )
    }

    // test "name" (x in gen, opt = v) { ... }
    fn test_block(&mut self) -> PResult<StmtKind> {
        let Tok::Ident(w) = self.bump().tok else { unreachable!() };
        let kind = TestKind::from_word(&w).unwrap();
        let Tok::Str(name) = self.bump().tok else { unreachable!() };
        let (mut gens, mut opts) = (Vec::new(), Vec::new());
        if self.eat(&Tok::LParen) {
            while !self.at(&Tok::RParen) {
                let (n, span) = self.ident("`name in generator` or `option = value`")?;
                if self.eat(&Tok::In) {
                    gens.push((n, self.expr()?));
                } else if self.eat(&Tok::Assign) {
                    opts.push((n, self.expr()?));
                } else {
                    return Err(Diag::new(format!("write `{n} in generator` or `{n} = value`"), span));
                }
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RParen, "`)`")?;
        }
        Ok(StmtKind::TestBlock { kind, name, gens, opts, body: self.block()? })
    }

    // expression, assignment, or unpack
    fn simple(&mut self) -> PResult<StmtKind> {
        let first = self.expr()?;
        let aug = match self.peek() {
            Tok::PlusEq => Some(BinOp::Add),
            Tok::MinusEq => Some(BinOp::Sub),
            Tok::StarEq => Some(BinOp::Mul),
            Tok::SlashEq => Some(BinOp::Div),
            Tok::PercentEq => Some(BinOp::Mod),
            _ => None,
        };
        if let Some(op) = aug {
            self.bump();
            check_target(&first)?;
            return Ok(StmtKind::AugAssign(op, first, self.expr()?));
        }
        let mut targets = vec![first];
        while self.eat(&Tok::Comma) {
            targets.push(self.expr()?);
        }
        if self.eat(&Tok::Assign) {
            for t in &targets {
                check_target(t)?;
            }
            let value = self.expr_list()?;
            return Ok(StmtKind::Assign(targets, value));
        }
        if targets.len() > 1 {
            return Err(self.unexpected("`=` after names"));
        }
        if matches!(self.peek(), Tok::Ident(_)) && matches!(targets[0].kind, ExprKind::Name(_)) {
            return Err(Diag::new("two expressions on one line", self.span()).note("put them on separate lines or join with `;`"));
        }
        Ok(StmtKind::Expr(targets.pop().unwrap()))
    }

    // `a, b, c` becomes a list
    fn expr_list(&mut self) -> PResult<Expr> {
        let first = self.expr()?;
        if !self.at(&Tok::Comma) {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat(&Tok::Comma) {
            items.push(self.expr()?);
        }
        let span = items[0].span.to(items.last().unwrap().span);
        Ok(Expr { kind: ExprKind::List(items), span })
    }

    fn fn_decl(&mut self, start: Span) -> PResult<Rc<FnDecl>> {
        let (name, _) = self.ident("function name")?;
        let params = self.params()?;
        let body = self.block()?;
        Ok(Rc::new(FnDecl { name, params, body, span: start.to(self.prev_span()) }))
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect(Tok::LParen, "`(`")?;
        let mut out: Vec<Param> = Vec::new();
        while !self.at(&Tok::RParen) {
            let (name, span) = self.ident("parameter name")?;
            if out.iter().any(|p| p.name == name) {
                return Err(Diag::new(format!("parameter `{name}` listed twice"), span));
            }
            let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
            if default.is_none() && out.last().is_some_and(|p| p.default.is_some()) {
                return Err(Diag::new(format!("parameter `{name}` needs a default because the one before it has one"), span));
            }
            out.push(Param { name, default, span });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(out)
    }

    fn class(&mut self) -> PResult<StmtKind> {
        self.bump();
        let (name, _) = self.ident("class name")?;
        let sup = if self.eat(&Tok::Colon) { Some(self.expr()?) } else { None };
        self.expect(Tok::LBrace, "`{`")?;
        let mut methods: Vec<Rc<FnDecl>> = Vec::new();
        loop {
            self.skip_seps();
            if self.eat(&Tok::RBrace) {
                break;
            }
            let start = self.span();
            if !self.eat(&Tok::Fn) {
                return Err(self.unexpected("`fn` (classes hold methods only)"));
            }
            let m = self.fn_decl(start)?;
            if methods.iter().any(|o| o.name == m.name) {
                return Err(Diag::new(format!("method `{}` defined twice", m.name), m.span));
            }
            methods.push(m);
        }
        Ok(StmtKind::Class { name, sup, methods })
    }

    fn if_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let mut arms = vec![(self.expr()?, self.block()?)];
        let mut other = None;
        loop {
            if self.next_line_is(&Tok::Elif) {
                self.bump();
                arms.push((self.expr()?, self.block()?));
            } else if self.next_line_is(&Tok::Else) {
                self.bump();
                if self.at(&Tok::If) {
                    return Err(Diag::new("write `elif`, not `else if`", self.span()));
                }
                other = Some(self.block()?);
                break;
            } else {
                break;
            }
        }
        Ok(StmtKind::If(arms, other))
    }

    fn for_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let mut vars = vec![self.ident("loop variable")?];
        while self.eat(&Tok::Comma) {
            vars.push(self.ident("loop variable")?);
        }
        self.expect(Tok::In, "`in`")?;
        let iter = self.expr()?;
        Ok(StmtKind::For(vars, iter, self.block()?))
    }

    fn import(&mut self) -> PResult<StmtKind> {
        self.bump();
        let mut out = Vec::new();
        loop {
            let span = self.span();
            let what = match self.peek().clone() {
                Tok::Ident(n) => {
                    self.bump();
                    Import::Builtin(n)
                }
                Tok::Str(p) => {
                    self.bump();
                    Import::File(p)
                }
                _ => return Err(self.unexpected("module name or \"file.mpp\"")),
            };
            let alias = if self.eat(&Tok::As) { Some(self.ident("alias")?.0) } else { None };
            out.push((what, alias, span.to(self.prev_span())));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(StmtKind::Import(out))
    }

    fn try_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        let body = self.block()?;
        if !self.next_line_is(&Tok::Catch) {
            return Err(self.unexpected("`catch` after `try` block"));
        }
        self.bump();
        let var = match self.peek().clone() {
            Tok::Ident(n) => Some((n, self.bump().span)),
            _ => None,
        };
        Ok(StmtKind::Try(body, var, self.block()?))
    }

    // ---- expressions ----

    pub fn expr(&mut self) -> PResult<Expr> {
        let cond = self.or_expr()?;
        if !self.eat(&Tok::Question) {
            return Ok(cond);
        }
        self.skip_newlines();
        let a = self.expr()?;
        self.skip_newlines();
        self.expect(Tok::Colon, "`:` in `cond ? a : b`")?;
        self.skip_newlines();
        let b = self.expr()?;
        let span = cond.span.to(b.span);
        Ok(Expr { kind: ExprKind::Ternary(Box::new(cond), Box::new(a), Box::new(b)), span })
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let mut l = self.and_expr()?;
        while self.eat(&Tok::Or) {
            self.skip_newlines();
            let r = self.and_expr()?;
            let span = l.span.to(r.span);
            l = Expr { kind: ExprKind::Or(Box::new(l), Box::new(r)), span };
        }
        Ok(l)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let mut l = self.not_expr()?;
        while self.eat(&Tok::And) {
            self.skip_newlines();
            let r = self.not_expr()?;
            let span = l.span.to(r.span);
            l = Expr { kind: ExprKind::And(Box::new(l), Box::new(r)), span };
        }
        Ok(l)
    }

    fn not_expr(&mut self) -> PResult<Expr> {
        if self.at(&Tok::Not) {
            let s = self.bump().span;
            let e = self.not_expr()?;
            let span = s.to(e.span);
            return Ok(Expr { kind: ExprKind::Unary(UnOp::Not, Box::new(e)), span });
        }
        self.comparison()
    }

    fn comp_op(&self) -> Option<(BinOp, usize)> {
        Some(match self.peek() {
            Tok::EqEq => (BinOp::Eq, 1),
            Tok::Ne => (BinOp::Ne, 1),
            Tok::Lt => (BinOp::Lt, 1),
            Tok::Le => (BinOp::Le, 1),
            Tok::Gt => (BinOp::Gt, 1),
            Tok::Ge => (BinOp::Ge, 1),
            Tok::In => (BinOp::In, 1),
            Tok::Approx => (BinOp::Approx, 1),
            Tok::Not if self.peek_at(1) == &Tok::In => (BinOp::NotIn, 2),
            _ => return None,
        })
    }

    fn comparison(&mut self) -> PResult<Expr> {
        let l = self.range_expr()?;
        let Some((op, n)) = self.comp_op() else {
            return Ok(l);
        };
        for _ in 0..n {
            self.bump();
        }
        self.skip_newlines();
        let r = self.range_expr()?;
        if self.comp_op().is_some() {
            return Err(Diag::new("chained comparison is not allowed", self.span()).note("write `a < b and b < c`"));
        }
        let span = l.span.to(r.span);
        Ok(Expr { kind: ExprKind::Binary(op, Box::new(l), Box::new(r)), span })
    }

    fn range_expr(&mut self) -> PResult<Expr> {
        let l = self.additive()?;
        let incl = match self.peek() {
            Tok::DotDot => false,
            Tok::DotDotEq => true,
            _ => return Ok(l),
        };
        self.bump();
        let r = self.additive()?;
        let span = l.span.to(r.span);
        Ok(Expr { kind: ExprKind::Range(Box::new(l), Box::new(r), incl), span })
    }

    fn additive(&mut self) -> PResult<Expr> {
        let mut l = self.mult()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(l),
            };
            self.bump();
            self.skip_newlines();
            let r = self.mult()?;
            let span = l.span.to(r.span);
            l = Expr { kind: ExprKind::Binary(op, Box::new(l), Box::new(r)), span };
        }
    }

    fn mult(&mut self) -> PResult<Expr> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::SlashSlash => BinOp::IntDiv,
                Tok::Percent => BinOp::Mod,
                _ => return Ok(l),
            };
            self.bump();
            self.skip_newlines();
            let r = self.unary()?;
            let span = l.span.to(r.span);
            l = Expr { kind: ExprKind::Binary(op, Box::new(l), Box::new(r)), span };
        }
    }

    fn unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Tok::Minus => {
                let s = self.bump().span;
                let e = self.unary()?;
                let span = s.to(e.span);
                // fold literal negatives so `-9223372036854775808` style edge stays simple
                let kind = match e.kind {
                    ExprKind::Int(n) => ExprKind::Int(-n),
                    ExprKind::Float(f) => ExprKind::Float(-f),
                    k => ExprKind::Unary(UnOp::Neg, Box::new(Expr { kind: k, span: e.span })),
                };
                Ok(Expr { kind, span })
            }
            Tok::Plus => {
                self.bump();
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> PResult<Expr> {
        let base = self.postfix()?;
        if !self.eat(&Tok::StarStar) {
            return Ok(base);
        }
        let exp = self.unary()?;
        let span = base.span.to(exp.span);
        Ok(Expr { kind: ExprKind::Binary(BinOp::Pow, Box::new(base), Box::new(exp)), span })
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            match self.peek() {
                Tok::LParen => {
                    self.bump();
                    let args = self.args()?;
                    let span = e.span.to(self.prev_span());
                    e = Expr { kind: ExprKind::Call(Box::new(e), args), span };
                }
                Tok::Dot => {
                    self.bump();
                    let (name, ns) = self.ident("name after `.`")?;
                    let span = e.span.to(ns);
                    e = Expr { kind: ExprKind::Prop(Box::new(e), name), span };
                }
                Tok::LBracket => {
                    self.bump();
                    let lo = if self.at(&Tok::Colon) { None } else { Some(Box::new(self.expr()?)) };
                    let kind = if self.eat(&Tok::Colon) {
                        let hi = if self.at(&Tok::RBracket) { None } else { Some(Box::new(self.expr()?)) };
                        ExprKind::Slice(Box::new(e.clone()), lo, hi)
                    } else {
                        ExprKind::Index(Box::new(e.clone()), lo.unwrap())
                    };
                    self.expect(Tok::RBracket, "`]`")?;
                    e = Expr { kind, span: e.span.to(self.prev_span()) };
                }
                _ => return Ok(e),
            }
        }
    }

    fn args(&mut self) -> PResult<Vec<Arg>> {
        let mut out: Vec<Arg> = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&Tok::RParen) {
                return Ok(out);
            }
            let name = match (self.peek().clone(), self.peek_at(1)) {
                (Tok::Ident(n), Tok::Assign) => {
                    let s = self.bump().span;
                    self.bump();
                    if out.iter().any(|a| a.name.as_ref() == Some(&n)) {
                        return Err(Diag::new(format!("argument `{n}` given twice"), s));
                    }
                    Some(n)
                }
                _ => {
                    if out.iter().any(|a| a.name.is_some()) {
                        return Err(Diag::new("plain argument after a named one", self.span()));
                    }
                    None
                }
            };
            out.push(Arg { name, value: self.expr()? });
            self.skip_newlines();
            if !self.eat(&Tok::Comma) {
                self.skip_newlines();
                self.expect(Tok::RParen, "`,` or `)`")?;
                return Ok(out);
            }
        }
    }

    // `(a, b) =>` ahead?
    fn paren_lambda_ahead(&self) -> bool {
        let mut depth = 0;
        let mut i = self.pos;
        loop {
            match &self.toks[i].tok {
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return self.toks.get(i + 1).map(|t| &t.tok) == Some(&Tok::Arrow);
                    }
                }
                Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    fn lambda_body(&mut self, params: Vec<Param>, start: Span) -> PResult<Expr> {
        self.expect(Tok::Arrow, "`=>`")?;
        let body = if self.at(&Tok::LBrace) {
            self.block()?
        } else {
            let e = self.expr()?;
            let span = e.span;
            vec![Stmt { kind: StmtKind::Return(Some(e)), span }]
        };
        let span = start.to(self.prev_span());
        Ok(Expr { kind: ExprKind::Lambda(Rc::new(FnDecl { name: "<lambda>".into(), params, body, span })), span })
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        let simple = |k| Ok(Expr { kind: k, span: start });
        match self.peek().clone() {
            Tok::Int(n) => {
                self.bump();
                simple(ExprKind::Int(n))
            }
            Tok::Float(f) => {
                self.bump();
                simple(ExprKind::Float(f))
            }
            Tok::Str(s) => {
                self.bump();
                simple(ExprKind::Str(s))
            }
            Tok::FStr(pieces) => {
                self.bump();
                let mut parts = Vec::new();
                for p in pieces {
                    parts.push(match p {
                        FPiece::Lit(s) => FPart::Lit(s),
                        FPiece::Expr { src, offset, spec } => {
                            let toks = lex(&src, offset)?;
                            FPart::Expr(Box::new(Parser::new(toks).lone_expr()?), spec)
                        }
                    });
                }
                simple(ExprKind::FStr(parts))
            }
            Tok::Nil => {
                self.bump();
                simple(ExprKind::Nil)
            }
            Tok::True | Tok::False => {
                let b = self.bump().tok == Tok::True;
                simple(ExprKind::Bool(b))
            }
            Tok::Ident(n) => {
                self.bump();
                if self.at(&Tok::Arrow) {
                    let p = Param { name: n, default: None, span: start };
                    return self.lambda_body(vec![p], start);
                }
                simple(ExprKind::Name(n))
            }
            Tok::Super => {
                self.bump();
                self.expect(Tok::Dot, "`.` after `super`")?;
                let (name, ns) = self.ident("method name")?;
                Ok(Expr { kind: ExprKind::Super(name), span: start.to(ns) })
            }
            Tok::Fn => {
                self.bump();
                let params = self.params()?;
                let body = self.block()?;
                let span = start.to(self.prev_span());
                Ok(Expr { kind: ExprKind::Lambda(Rc::new(FnDecl { name: "<fn>".into(), params, body, span })), span })
            }
            Tok::LParen => {
                if self.paren_lambda_ahead() {
                    let params = self.params()?;
                    return self.lambda_body(params, start);
                }
                self.bump();
                let mut e = self.expr()?;
                self.expect(Tok::RParen, "`)`")?;
                e.span = start.to(self.prev_span());
                Ok(e)
            }
            Tok::LBracket => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    self.skip_newlines();
                    if self.eat(&Tok::RBracket) {
                        break;
                    }
                    items.push(self.expr()?);
                    self.skip_newlines();
                    if !self.eat(&Tok::Comma) {
                        self.skip_newlines();
                        self.expect(Tok::RBracket, "`,` or `]`")?;
                        break;
                    }
                }
                Ok(Expr { kind: ExprKind::List(items), span: start.to(self.prev_span()) })
            }
            Tok::LBrace => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    self.skip_newlines();
                    if self.eat(&Tok::RBrace) {
                        break;
                    }
                    let k = self.expr()?;
                    self.expect(Tok::Colon, "`:` in map")?;
                    self.skip_newlines();
                    let v = self.expr()?;
                    items.push((k, v));
                    self.skip_newlines();
                    if !self.eat(&Tok::Comma) {
                        self.skip_newlines();
                        self.expect(Tok::RBrace, "`,` or `}`")?;
                        break;
                    }
                }
                Ok(Expr { kind: ExprKind::Map(items), span: start.to(self.prev_span()) })
            }
            _ => Err(self.unexpected("expression")),
        }
    }
}

fn check_target(e: &Expr) -> PResult<()> {
    match e.kind {
        ExprKind::Name(_) | ExprKind::Prop(..) | ExprKind::Index(..) => Ok(()),
        _ => Err(Diag::new("cannot assign to this", e.span).note("assign to a name, `obj.field` or `list[i]`")),
    }
}

#[cfg(test)]
mod tests {
    use crate::syntax::parse;

    #[test]
    fn parses_everything() {
        let src = r#"
import math, "lib/h.mpp" as h
const N = 10
fn f(a, b = 2) { return a + b * 2 ** 3 }
class P : Base {
    fn init(self, x) { self.x = x }
    fn get(self) { return super.get() }
}
g = (a, b) => a + b
h2 = x => { return x }
xs = [1, 2,
      3]
m = {"a": 1,
     "b": 2}
for k, v in m.items() { print(k, v) }
if a < b { x = 1 }
elif a > b { x = 2 }
else { x = 3 }
try { throw "bad" } catch e { print(e.message) }
y = c ? 1 : 2
z = xs[1:] + xs[:2]
print(f"v={x:.2f}")
w = data
    .filter(r => r.ok)
    .len()
a, b = b, a
n += 1
ok = 3 not in xs and not false
"#;
        if let Err(d) = parse(src) {
            panic!("{d:?}");
        }
    }

    #[test]
    fn reports_many_errors() {
        let errs = parse("x = \nz = 1 1\nw = 3\n").unwrap_err();
        assert!(errs.len() >= 2, "{errs:?}");
    }

    #[test]
    fn rejects_chains_and_bad_targets() {
        assert!(parse("a < b < c").is_err());
        assert!(parse("f() = 3").is_err());
        assert!(parse("if x { } else if y { }").is_err());
    }
}
