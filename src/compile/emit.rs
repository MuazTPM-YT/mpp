use super::chunk::*;
use super::join_key;
use crate::diag::LineIndex;
use crate::syntax::ast::*;
use crate::syntax::{Diag, Span};
use indexmap::IndexMap;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Module,
    Function,
    Method,
}

enum Res {
    Local(u16),
    Free(u16),
    Global(u32),
    Builtin(u16),
}

struct Loop {
    start: usize,
    breaks: Vec<usize>,
    try_depth: u32,
}

struct FnState {
    name: Rc<str>,
    kind: Kind,
    params: Vec<Rc<str>>,
    required: u16,
    locals: IndexMap<Rc<str>, u16>,
    cells: Vec<u16>,
    frees: Vec<(Rc<str>, Capture)>,
    global_decl: HashSet<Rc<str>>,
    nonlocal_decl: HashSet<Rc<str>>,
    consts: HashSet<Rc<str>>,
    code: Vec<Op>,
    pool: Vec<Const>,
    lines: Vec<(u32, u32)>,
    loops: Vec<Loop>,
    try_depth: u32,
    hidden: u32,
}

impl FnState {
    fn new(name: Rc<str>, kind: Kind) -> FnState {
        FnState {
            name,
            kind,
            params: Vec::new(),
            required: 0,
            locals: IndexMap::new(),
            cells: Vec::new(),
            frees: Vec::new(),
            global_decl: HashSet::new(),
            nonlocal_decl: HashSet::new(),
            consts: HashSet::new(),
            code: Vec::new(),
            pool: Vec::new(),
            lines: Vec::new(),
            loops: Vec::new(),
            try_depth: 0,
            hidden: 0,
        }
    }

    fn local(&mut self, name: &Rc<str>) -> u16 {
        let n = self.locals.len() as u16;
        *self.locals.entry(name.clone()).or_insert(n)
    }
}

pub struct ModuleOut {
    pub proto: ModuleProto,
    // file imports found: (module key, span of import)
    pub imports: Vec<(Rc<str>, Span)>,
}

pub struct Compiler<'a> {
    key: Rc<str>,
    src: &'a str,
    lines: LineIndex,
    fns: Vec<FnState>,
    globals: IndexMap<Rc<str>, u32>,
    diags: Vec<Diag>,
    imports: Vec<(Rc<str>, Span)>,
    span: Span,
}

// compile one module; `prev_globals` keeps REPL state between lines
pub fn compile_module(stmts: &[Stmt], key: &str, src: &str, prev_globals: &[Rc<str>], repl: bool) -> Result<ModuleOut, Vec<Diag>> {
    let mut c = Compiler {
        key: key.into(),
        src,
        lines: LineIndex::new(src),
        fns: vec![FnState::new("<module>".into(), Kind::Module)],
        globals: IndexMap::new(),
        diags: Vec::new(),
        imports: Vec::new(),
        span: Span::default(),
    };
    for g in prev_globals {
        c.global_slot(g);
    }
    let mut assigned = Vec::new();
    let mut decl = Vec::new();
    prescan(stmts, &mut assigned, &mut decl);
    for n in &assigned {
        c.global_slot(n);
    }
    let n = stmts.len();
    for (i, s) in stmts.iter().enumerate() {
        if let (true, StmtKind::Expr(e)) = (repl && i + 1 == n, &s.kind) {
            c.span = s.span;
            c.expr(e);
            c.emit(Op::ReplPrint);
            continue;
        }
        c.stmt(s);
    }
    c.emit(Op::Nil);
    c.emit(Op::Return);
    if !c.diags.is_empty() {
        return Err(c.diags);
    }
    let f = c.fns.pop().unwrap();
    let main = c.finish(f);
    Ok(ModuleOut { proto: ModuleProto { key: c.key.clone(), globals: c.globals.keys().cloned().collect(), main }, imports: c.imports })
}

// names bound by assignment in a body (not inside nested functions)
fn prescan(stmts: &[Stmt], out: &mut Vec<Rc<str>>, decl: &mut Vec<(Rc<str>, bool)>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Assign(targets, _) => {
                for t in targets {
                    if let ExprKind::Name(n) = &t.kind {
                        out.push(n.clone());
                    }
                }
            }
            StmtKind::AugAssign(_, t, _) => {
                if let ExprKind::Name(n) = &t.kind {
                    out.push(n.clone());
                }
            }
            StmtKind::Const(n, _) => out.push(n.clone()),
            StmtKind::Fn(f) => out.push(f.name.clone()),
            StmtKind::Class { name, .. } => out.push(name.clone()),
            StmtKind::Import(items) => {
                for (what, alias, _) in items {
                    if let Some(n) = alias.clone().or_else(|| import_name(what)) {
                        out.push(n);
                    }
                }
            }
            StmtKind::If(arms, other) => {
                for (_, b) in arms {
                    prescan(b, out, decl);
                }
                if let Some(b) = other {
                    prescan(b, out, decl);
                }
            }
            StmtKind::While(_, b) => prescan(b, out, decl),
            StmtKind::For(vars, _, b) => {
                out.extend(vars.iter().map(|v| v.0.clone()));
                prescan(b, out, decl);
            }
            StmtKind::Try(b, var, cb) => {
                prescan(b, out, decl);
                if let Some(v) = var {
                    out.push(v.0.clone());
                }
                prescan(cb, out, decl);
            }
            StmtKind::Global(ns) => decl.extend(ns.iter().map(|n| (n.clone(), true))),
            StmtKind::Nonlocal(ns) => decl.extend(ns.iter().map(|n| (n.clone(), false))),
            _ => {}
        }
    }
}

// default binding name for an import
fn import_name(what: &Import) -> Option<Rc<str>> {
    match what {
        Import::Builtin(n) => Some(n.clone()),
        Import::File(p) => {
            let stem = std::path::Path::new(&**p).file_stem()?.to_str()?;
            let ok =
                stem.chars().next().is_some_and(|c| c == '_' || c.is_alphabetic()) && stem.chars().all(|c| c == '_' || c.is_alphanumeric());
            ok.then(|| stem.into())
        }
    }
}

fn binop(op: BinOp) -> Op {
    match op {
        BinOp::Add => Op::Add,
        BinOp::Sub => Op::Sub,
        BinOp::Mul => Op::Mul,
        BinOp::Div => Op::Div,
        BinOp::IntDiv => Op::IntDiv,
        BinOp::Mod => Op::Mod,
        BinOp::Pow => Op::Pow,
        BinOp::Eq => Op::Eq,
        BinOp::Ne => Op::Ne,
        BinOp::Lt => Op::Lt,
        BinOp::Le => Op::Le,
        BinOp::Gt => Op::Gt,
        BinOp::Ge => Op::Ge,
        BinOp::In => Op::In,
        BinOp::NotIn => Op::NotIn,
        BinOp::Approx => Op::Approx,
    }
}

impl Compiler<'_> {
    fn f(&mut self) -> &mut FnState {
        self.fns.last_mut().unwrap()
    }

    fn err(&mut self, msg: impl Into<String>, span: Span) {
        self.diags.push(Diag::new(msg, span));
    }

    fn emit(&mut self, op: Op) -> usize {
        let lc = self.lines.line_col(self.src, self.span.start);
        let f = self.f();
        f.code.push(op);
        f.lines.push(lc);
        f.code.len() - 1
    }

    fn here(&mut self) -> usize {
        self.f().code.len()
    }

    // point jump at `to`
    fn patch(&mut self, at: usize, to: usize) {
        let to = to as u32;
        let op = &mut self.f().code[at];
        *op = match *op {
            Op::Jump(_) => Op::Jump(to),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(to),
            Op::JumpIfFalseKeep(_) => Op::JumpIfFalseKeep(to),
            Op::JumpIfTrueKeep(_) => Op::JumpIfTrueKeep(to),
            Op::IterNext(_) => Op::IterNext(to),
            Op::TryBegin(_) => Op::TryBegin(to),
            Op::DefaultArg(s, _) => Op::DefaultArg(s, to),
            other => other,
        };
    }

    fn patch_here(&mut self, at: usize) {
        let h = self.here();
        self.patch(at, h);
    }

    fn konst(&mut self, c: Const) -> u32 {
        let pool = &mut self.f().pool;
        let same = |a: &Const| match (a, &c) {
            (Const::Str(x), Const::Str(y)) => x == y,
            (Const::Int(x), Const::Int(y)) => x == y,
            (Const::Float(x), Const::Float(y)) => x.to_bits() == y.to_bits(),
            _ => false,
        };
        if let Some(i) = pool.iter().position(same) {
            return i as u32;
        }
        pool.push(c);
        (pool.len() - 1) as u32
    }

    fn name_const(&mut self, n: &Rc<str>) -> u32 {
        self.konst(Const::Str(n.clone()))
    }

    fn global_slot(&mut self, n: &Rc<str>) -> u32 {
        let len = self.globals.len() as u32;
        *self.globals.entry(n.clone()).or_insert(len)
    }

    fn hidden_local(&mut self) -> u16 {
        let f = self.f();
        f.hidden += 1;
        let name: Rc<str> = format!("%{}", f.hidden).into();
        f.local(&name)
    }

    fn finish(&mut self, f: FnState) -> Rc<FuncProto> {
        Rc::new(FuncProto {
            name: f.name,
            file: self.key.clone(),
            required: f.required,
            params: f.params,
            nlocals: f.locals.len() as u16,
            locals: f.locals.keys().cloned().collect(),
            cells: f.cells,
            captures: f.frees.iter().map(|(_, c)| *c).collect(),
            free_names: f.frees.iter().map(|(n, _)| n.clone()).collect(),
            code: f.code,
            consts: f.pool,
            lines: f.lines,
        })
    }

    // ---- name resolution ----

    fn resolve(&mut self, name: &Rc<str>) -> Option<Res> {
        let depth = self.fns.len() - 1;
        let f = &self.fns[depth];
        if f.kind != Kind::Module && !f.global_decl.contains(name) {
            if let (false, Some(&slot)) = (f.nonlocal_decl.contains(name), f.locals.get(name)) {
                return Some(Res::Local(slot));
            }
            if let Some(i) = self.resolve_free(depth, name) {
                return Some(Res::Free(i));
            }
        }
        if let Some(&g) = self.globals.get(name) {
            return Some(Res::Global(g));
        }
        crate::stdlib::builtin_index(name).map(Res::Builtin)
    }

    fn resolve_free(&mut self, depth: usize, name: &Rc<str>) -> Option<u16> {
        if let Some(i) = self.fns[depth].frees.iter().position(|(n, _)| n == name) {
            return Some(i as u16);
        }
        if depth == 0 || self.fns[depth - 1].kind == Kind::Module {
            return None;
        }
        let parent = &mut self.fns[depth - 1];
        if parent.global_decl.contains(name) {
            return None;
        }
        let cap = match parent.locals.get(name) {
            Some(&slot) if !parent.nonlocal_decl.contains(name) => {
                if !parent.cells.contains(&slot) {
                    parent.cells.push(slot);
                }
                Capture::Local(slot)
            }
            _ => Capture::Free(self.resolve_free(depth - 1, name)?),
        };
        let f = &mut self.fns[depth];
        f.frees.push((name.clone(), cap));
        Some((f.frees.len() - 1) as u16)
    }

    // is this name a const in the scope it resolves to?
    fn is_const(&self, name: &Rc<str>) -> bool {
        let f = self.fns.last().unwrap();
        if f.kind != Kind::Module && f.locals.contains_key(name) && !f.global_decl.contains(name) {
            return f.consts.contains(name);
        }
        self.fns[0].consts.contains(name) && !self.fns.iter().skip(1).any(|f| f.locals.contains_key(name) && !f.global_decl.contains(name))
    }

    fn load_name(&mut self, name: &Rc<str>, span: Span) {
        match self.resolve(name) {
            Some(Res::Local(s)) => self.emit(Op::LoadLocal(s)),
            Some(Res::Free(i)) => self.emit(Op::LoadFree(i)),
            Some(Res::Global(g)) => self.emit(Op::LoadGlobal(g)),
            Some(Res::Builtin(b)) => self.emit(Op::LoadBuiltin(b)),
            None => {
                let d = Diag::new(format!("`{name}` is not defined"), span);
                let d = match self.hint(name) {
                    Some(h) => d.note(h),
                    None => d,
                };
                self.diags.push(d);
                0
            }
        };
    }

    // "did you mean" help for an unknown name
    fn hint(&self, name: &str) -> Option<String> {
        if crate::stdlib::is_module(name) {
            return Some(format!("add `import {name}` at the top"));
        }
        for m in crate::stdlib::MODULES {
            if crate::stdlib::module(m).is_some_and(|mm| mm.contains_key(name)) {
                return Some(format!("`{name}` lives in module `{m}`: add `import {m}` and write `{m}.{name}`"));
            }
        }
        let mut seen: Vec<&str> = self.globals.keys().map(|k| &**k).collect();
        for f in &self.fns {
            seen.extend(f.locals.keys().filter(|k| !k.starts_with('%')).map(|k| &**k));
        }
        seen.extend(crate::stdlib::BUILTINS.iter().map(|b| b.name));
        let best = seen
            .into_iter()
            .map(|c| (edit_distance(name, c), c))
            .filter(|(d, c)| *d <= 2 && *d < name.len().max(c.len()))
            .min_by_key(|(d, _)| *d)?;
        Some(format!("did you mean `{}`?", best.1))
    }

    fn store_name(&mut self, name: &Rc<str>, span: Span) {
        if self.is_const(name) {
            self.err(format!("`{name}` is a const and cannot change"), span);
        }
        let in_fn = self.fns.last().unwrap().kind != Kind::Module;
        let f = self.fns.last().unwrap();
        let res = if in_fn && f.global_decl.contains(name) {
            Res::Global(self.global_slot(name))
        } else if in_fn && f.nonlocal_decl.contains(name) {
            match self.resolve_free(self.fns.len() - 1, name) {
                Some(i) => Res::Free(i),
                None => {
                    self.err(format!("nonlocal `{name}` is not a variable of an outer function"), span);
                    return;
                }
            }
        } else if in_fn {
            let slot = self.f().local(name);
            Res::Local(slot)
        } else {
            Res::Global(self.global_slot(name))
        };
        match res {
            Res::Local(s) => self.emit(Op::StoreLocal(s)),
            Res::Free(i) => self.emit(Op::StoreFree(i)),
            Res::Global(g) => self.emit(Op::StoreGlobal(g)),
            Res::Builtin(_) => unreachable!(),
        };
    }

    // ---- statements ----

    fn block(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        self.span = s.span;
        match &s.kind {
            StmtKind::Expr(e) => {
                self.expr(e);
                self.emit(Op::Pop);
            }
            StmtKind::Assign(targets, value) => self.assign(targets, value),
            StmtKind::AugAssign(op, target, value) => self.aug_assign(*op, target, value),
            StmtKind::Const(name, value) => {
                let f = self.fns.last().unwrap();
                if f.consts.contains(name) {
                    self.err(format!("const `{name}` declared twice"), s.span);
                }
                self.expr(value);
                self.store_name(name, s.span);
                self.f().consts.insert(name.clone());
            }
            StmtKind::If(arms, other) => {
                let mut ends = Vec::new();
                for (cond, body) in arms {
                    self.span = cond.span;
                    self.expr(cond);
                    let skip = self.emit(Op::JumpIfFalse(0));
                    self.block(body);
                    ends.push(self.emit(Op::Jump(0)));
                    self.patch_here(skip);
                }
                if let Some(b) = other {
                    self.block(b);
                }
                for e in ends {
                    self.patch_here(e);
                }
            }
            StmtKind::While(cond, body) => {
                let start = self.here();
                self.expr(cond);
                let exit = self.emit(Op::JumpIfFalse(0));
                self.loop_body(start, body);
                self.emit(Op::Jump(start as u32));
                self.patch_here(exit);
                self.end_loop();
            }
            StmtKind::For(vars, iter, body) => {
                self.expr(iter);
                self.emit(Op::IterInit);
                let it = self.hidden_local();
                self.emit(Op::StoreLocal(it));
                let start = self.here();
                self.emit(Op::LoadLocal(it));
                let exit = self.emit(Op::IterNext(0));
                if vars.len() > 1 {
                    self.emit(Op::Unpack(vars.len() as u32));
                }
                for (name, span) in vars.iter().rev() {
                    self.store_name(name, *span);
                }
                self.loop_body(start, body);
                self.emit(Op::Jump(start as u32));
                self.patch_here(exit);
                self.end_loop();
            }
            StmtKind::Break | StmtKind::Continue => {
                let Some(lp) = self.f().loops.last() else {
                    self.err("`break`/`continue` outside a loop", s.span);
                    return;
                };
                let (start, depth) = (lp.start, lp.try_depth);
                for _ in depth..self.f().try_depth {
                    self.emit(Op::TryEnd);
                }
                if matches!(s.kind, StmtKind::Break) {
                    let j = self.emit(Op::Jump(0));
                    self.f().loops.last_mut().unwrap().breaks.push(j);
                } else {
                    self.emit(Op::Jump(start as u32));
                }
            }
            StmtKind::Return(value) => {
                if self.f().kind == Kind::Module {
                    self.err("`return` outside a function", s.span);
                }
                match value {
                    Some(v) => self.expr(v),
                    None => {
                        self.emit(Op::Nil);
                    }
                }
                self.emit(Op::Return);
            }
            StmtKind::Fn(decl) => {
                self.function(decl, Kind::Function);
                self.store_name(&decl.name, s.span);
            }
            StmtKind::Class { name, sup, methods } => self.class(name, sup.as_ref(), methods, s.span),
            StmtKind::Import(items) => {
                for (what, alias, span) in items {
                    self.span = *span;
                    match what {
                        Import::Builtin(n) => {
                            if !crate::stdlib::is_module(n) {
                                self.err(format!("no built-in module named `{n}`"), *span);
                                continue;
                            }
                            let k = self.name_const(n);
                            self.emit(Op::ImportBuiltin(k));
                        }
                        Import::File(p) => {
                            let key: Rc<str> = join_key(&self.key, p).into();
                            self.imports.push((key.clone(), *span));
                            let k = self.konst(Const::Str(key));
                            self.emit(Op::Import(k));
                        }
                    }
                    match alias.clone().or_else(|| import_name(what)) {
                        Some(n) => self.store_name(&n, *span),
                        None => self.err("file name is not a valid name; add `as name`", *span),
                    }
                }
            }
            StmtKind::Try(body, var, catch) => {
                let begin = self.emit(Op::TryBegin(0));
                self.f().try_depth += 1;
                self.block(body);
                self.f().try_depth -= 1;
                self.emit(Op::TryEnd);
                let end = self.emit(Op::Jump(0));
                self.patch_here(begin);
                match var {
                    Some((n, sp)) => self.store_name(n, *sp),
                    None => {
                        self.emit(Op::Pop);
                    }
                }
                self.block(catch);
                self.patch_here(end);
            }
            StmtKind::Throw(e) => {
                self.expr(e);
                self.emit(Op::Throw);
            }
            StmtKind::Global(_) | StmtKind::Nonlocal(_) => {}
        }
    }

    fn loop_body(&mut self, start: usize, body: &[Stmt]) {
        let try_depth = self.f().try_depth;
        self.f().loops.push(Loop { start, breaks: Vec::new(), try_depth });
        self.block(body);
    }

    fn end_loop(&mut self) {
        let lp = self.f().loops.pop().unwrap();
        for b in lp.breaks {
            self.patch_here(b);
        }
    }

    fn assign(&mut self, targets: &[Expr], value: &Expr) {
        if targets.len() == 1 {
            let t = &targets[0];
            match &t.kind {
                ExprKind::Name(n) => {
                    self.expr(value);
                    self.store_name(n, t.span);
                }
                ExprKind::Prop(obj, name) => {
                    self.expr(obj);
                    self.expr(value);
                    let k = self.name_const(name);
                    self.emit(Op::SetProp(k));
                }
                ExprKind::Index(obj, idx) => {
                    self.expr(obj);
                    self.expr(idx);
                    self.expr(value);
                    self.emit(Op::SetIndex);
                }
                _ => unreachable!("parser checks targets"),
            }
            return;
        }
        self.expr(value);
        self.emit(Op::Unpack(targets.len() as u32));
        for t in targets.iter().rev() {
            match &t.kind {
                ExprKind::Name(n) => self.store_name(n, t.span),
                ExprKind::Prop(obj, name) => {
                    self.expr(obj);
                    self.emit(Op::Swap);
                    let k = self.name_const(name);
                    self.emit(Op::SetProp(k));
                }
                ExprKind::Index(obj, idx) => {
                    self.expr(obj);
                    self.expr(idx);
                    self.emit(Op::Rot3);
                    self.emit(Op::SetIndex);
                }
                _ => unreachable!("parser checks targets"),
            }
        }
    }

    fn aug_assign(&mut self, op: BinOp, target: &Expr, value: &Expr) {
        match &target.kind {
            ExprKind::Name(n) => {
                self.load_name(n, target.span);
                self.expr(value);
                self.emit(binop(op));
                self.store_name(n, target.span);
            }
            ExprKind::Prop(obj, name) => {
                self.expr(obj);
                self.emit(Op::Dup);
                let k = self.name_const(name);
                self.emit(Op::GetProp(k));
                self.expr(value);
                self.emit(binop(op));
                self.emit(Op::SetProp(k));
            }
            ExprKind::Index(obj, idx) => {
                self.expr(obj);
                self.expr(idx);
                self.emit(Op::Dup2);
                self.emit(Op::GetIndex);
                self.expr(value);
                self.emit(binop(op));
                self.emit(Op::SetIndex);
            }
            _ => unreachable!("parser checks targets"),
        }
    }

    // compile a function body; leaves a closure on the stack
    fn function(&mut self, decl: &FnDecl, kind: Kind) {
        let mut st = FnState::new(decl.name.clone(), kind);
        st.params = decl.params.iter().map(|p| p.name.clone()).collect();
        st.required = decl.params.iter().take_while(|p| p.default.is_none()).count() as u16;
        for p in &decl.params {
            st.local(&p.name);
        }
        let mut assigned = Vec::new();
        let mut decl_names = Vec::new();
        prescan(&decl.body, &mut assigned, &mut decl_names);
        for (n, global) in decl_names {
            if st.params.contains(&n) {
                self.err(format!("`{n}` is a parameter, it cannot be global/nonlocal"), decl.span);
            } else if global {
                st.global_decl.insert(n);
            } else {
                st.nonlocal_decl.insert(n);
            }
        }
        for n in assigned {
            if !st.global_decl.contains(&n) && !st.nonlocal_decl.contains(&n) {
                st.local(&n);
            }
        }
        if st.params.len() > 255 {
            self.err("more than 255 parameters", decl.span);
        }
        let outer_span = self.span;
        self.fns.push(st);
        for g in self.fns.last().unwrap().global_decl.clone() {
            self.global_slot(&g);
        }
        for (i, p) in decl.params.iter().enumerate() {
            if let Some(d) = &p.default {
                self.span = d.span;
                let j = self.emit(Op::DefaultArg(i as u16, 0));
                self.expr(d);
                self.emit(Op::StoreLocal(i as u16));
                self.patch_here(j);
            }
        }
        self.block(&decl.body);
        self.span = Span::new(decl.span.end.saturating_sub(1) as usize, decl.span.end as usize);
        self.emit(Op::Nil);
        self.emit(Op::Return);
        let st = self.fns.pop().unwrap();
        let proto = self.finish(st);
        self.span = outer_span;
        let k = self.konst(Const::Func(proto));
        self.emit(Op::Closure(k));
    }

    fn class(&mut self, name: &Rc<str>, sup: Option<&Expr>, methods: &[Rc<FnDecl>], span: Span) {
        if let Some(s) = sup {
            self.expr(s);
        }
        let k = self.name_const(name);
        self.span = span;
        self.emit(Op::Class(k, sup.is_some()));
        for m in methods {
            if m.params.is_empty() {
                self.err(format!("method `{}` needs `self` as first parameter", m.name), m.span);
                continue;
            }
            self.function(m, Kind::Method);
            let mk = self.name_const(&m.name);
            self.emit(Op::Method(mk));
        }
        self.store_name(name, span);
    }

    // ---- expressions ----

    fn expr(&mut self, e: &Expr) {
        let saved = self.span;
        self.span = e.span;
        self.expr_inner(e);
        self.span = saved;
    }

    fn expr_inner(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Nil => {
                self.emit(Op::Nil);
            }
            ExprKind::Bool(b) => {
                self.emit(if *b { Op::True } else { Op::False });
            }
            ExprKind::Int(n) => {
                let k = self.konst(Const::Int(*n));
                self.emit(Op::Const(k));
            }
            ExprKind::Float(x) => {
                let k = self.konst(Const::Float(*x));
                self.emit(Op::Const(k));
            }
            ExprKind::Str(s) => {
                let k = self.konst(Const::Str(s.clone()));
                self.emit(Op::Const(k));
            }
            ExprKind::FStr(parts) => {
                if parts.is_empty() {
                    let k = self.konst(Const::Str("".into()));
                    self.emit(Op::Const(k));
                    return;
                }
                for p in parts {
                    match p {
                        FPart::Lit(s) => {
                            let k = self.konst(Const::Str(s.as_str().into()));
                            self.emit(Op::Const(k));
                        }
                        FPart::Expr(x, spec) => {
                            self.expr(x);
                            let k = match spec {
                                Some(s) => self.konst(Const::Str(s.as_str().into())),
                                None => NO_SPEC,
                            };
                            self.emit(Op::Format(k));
                        }
                    }
                }
                if parts.len() > 1 {
                    self.emit(Op::Concat(parts.len() as u32));
                }
            }
            ExprKind::Name(n) => self.load_name(n, e.span),
            ExprKind::List(items) => {
                for i in items {
                    self.expr(i);
                }
                self.emit(Op::MakeList(items.len() as u32));
            }
            ExprKind::Map(items) => {
                for (k, v) in items {
                    self.expr(k);
                    self.expr(v);
                }
                self.emit(Op::MakeMap(items.len() as u32));
            }
            ExprKind::Unary(op, x) => {
                self.expr(x);
                self.emit(if *op == UnOp::Neg { Op::Neg } else { Op::Not });
            }
            ExprKind::Binary(op, l, r) => {
                self.expr(l);
                self.expr(r);
                self.emit(binop(*op));
            }
            ExprKind::And(l, r) | ExprKind::Or(l, r) => {
                self.expr(l);
                let j = if matches!(e.kind, ExprKind::And(..)) { Op::JumpIfFalseKeep(0) } else { Op::JumpIfTrueKeep(0) };
                let j = self.emit(j);
                self.expr(r);
                self.patch_here(j);
            }
            ExprKind::Ternary(c, a, b) => {
                self.expr(c);
                let skip = self.emit(Op::JumpIfFalse(0));
                self.expr(a);
                let end = self.emit(Op::Jump(0));
                self.patch_here(skip);
                self.expr(b);
                self.patch_here(end);
            }
            ExprKind::Call(callee, args) => self.call(callee, args),
            ExprKind::Prop(obj, name) => {
                self.expr(obj);
                let k = self.name_const(name);
                self.emit(Op::GetProp(k));
            }
            ExprKind::Index(obj, idx) => {
                self.expr(obj);
                self.expr(idx);
                self.emit(Op::GetIndex);
            }
            ExprKind::Slice(obj, lo, hi) => {
                self.expr(obj);
                for part in [lo, hi] {
                    match part {
                        Some(x) => self.expr(x),
                        None => {
                            self.emit(Op::Nil);
                        }
                    }
                }
                self.emit(Op::GetSlice);
            }
            ExprKind::Range(lo, hi, incl) => {
                self.expr(lo);
                self.expr(hi);
                self.emit(Op::MakeRange(*incl));
            }
            ExprKind::Lambda(decl) => self.function(decl, Kind::Function),
            ExprKind::Super(name) => self.super_get(name, e.span),
        }
    }

    fn super_get(&mut self, name: &Rc<str>, span: Span) {
        if self.f().kind != Kind::Method {
            self.err("`super` only works directly inside a method", span);
            return;
        }
        self.emit(Op::LoadLocal(0));
        let k = self.name_const(name);
        self.emit(Op::GetSuper(k));
    }

    fn call(&mut self, callee: &Expr, args: &[Arg]) {
        if args.len() > 255 {
            self.err("more than 255 arguments", callee.span);
            return;
        }
        let kw: Vec<Rc<str>> = args.iter().filter_map(|a| a.name.clone()).collect();
        let kwk = if kw.is_empty() { None } else { Some(self.konst(Const::Names(kw.into()))) };
        let argc = args.len() as u8;
        if let ExprKind::Prop(obj, name) = &callee.kind {
            self.expr(obj);
            for a in args {
                self.expr(&a.value);
            }
            let k = self.name_const(name);
            match kwk {
                Some(n) => self.emit(Op::InvokeKw(k, argc, n)),
                None => self.emit(Op::Invoke(k, argc)),
            };
            return;
        }
        self.expr(callee);
        for a in args {
            self.expr(&a.value);
        }
        match kwk {
            Some(n) => self.emit(Op::CallKw(argc, n)),
            None => self.emit(Op::Call(argc)),
        };
    }
}

// levenshtein distance, small strings only
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = (a[i - 1] != b[j - 1]) as usize;
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}
