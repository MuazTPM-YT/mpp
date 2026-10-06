pub mod ops;
pub mod value;

use crate::compile::chunk::{Capture, Const, ModuleProto, NO_SPEC, Op, Program};
use crate::stdlib;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;
pub use value::*;

const MAX_FRAMES: usize = 20_000;
const MAX_NATIVE_DEPTH: usize = 400;

struct Frame {
    closure: Rc<Closure>,
    ip: usize,
    base: usize,
    // stack length to cut back to on return
    ret_to: usize,
    // `init` returns the new instance, not its own value
    init_self: Option<Value>,
}

struct Handler {
    frame: usize,
    stack: usize,
    ip: usize,
}

enum ModState {
    Loading,
    Done(Rc<Module>),
}

pub struct Vm {
    pub stack: Vec<Value>,
    frames: Vec<Frame>,
    handlers: Vec<Handler>,
    pub program: Program,
    modules: HashMap<Rc<str>, ModState>,
    builtin_mods: HashMap<Rc<str>, Rc<Module>>,
    pub out: Box<dyn Write>,
    pub rng: stdlib::rand::Rng,
    pub argv: Vec<String>,
    native_depth: usize,
    repl_globals: Option<Rc<Globals>>,
    // test blocks seen while running module code
    pub tests: Vec<TestDef>,
    // set while a test runs
    pub test_ctx: Option<TestCtx>,
    // stop with TimeoutError after this moment
    pub deadline: Option<std::time::Instant>,
}

pub struct TestDef {
    pub kind: u8,
    pub name: Rc<str>,
    pub func: Value,
    pub opts: Value,
    pub gens: Vec<Value>,
    pub file: Rc<str>,
    pub line: u32,
}

#[derive(Default)]
pub struct TestCtx {
    pub name: String,
    pub reports: Vec<(String, Value)>,
    pub snap_index: usize,
    pub snapshots: Option<Rc<RefCell<crate::runner::Snapshots>>>,
    pub update_snapshots: bool,
    pub notes: Vec<String>,
}

impl Vm {
    pub fn new(program: Program, out: Box<dyn Write>) -> Vm {
        Vm {
            stack: Vec::with_capacity(1024),
            frames: Vec::new(),
            handlers: Vec::new(),
            program,
            modules: HashMap::new(),
            builtin_mods: HashMap::new(),
            out,
            rng: stdlib::rand::Rng::new(0),
            argv: Vec::new(),
            native_depth: 0,
            repl_globals: None,
            tests: Vec::new(),
            test_ctx: None,
            deadline: None,
        }
    }

    // run the entry module
    pub fn run_main(&mut self) -> Result<Rc<Module>, Flow> {
        let entry = self.program.entry.clone();
        let r = self.load_module(&entry);
        let _ = self.out.flush();
        r
    }

    // run one REPL line; globals live across lines
    pub fn run_repl(&mut self, proto: ModuleProto) -> Result<(), Flow> {
        let g = self
            .repl_globals
            .get_or_insert_with(|| Rc::new(Globals { names: RefCell::new(Vec::new()), vals: RefCell::new(Vec::new()) }))
            .clone();
        *g.names.borrow_mut() = proto.globals.clone();
        g.vals.borrow_mut().resize(proto.globals.len(), Value::Undef);
        let c = Rc::new(Closure { proto: proto.main.clone(), frees: Vec::new(), globals: g, home: RefCell::new(None) });
        let r = self.call(&Value::Func(c), &[]).map(|_| ());
        let _ = self.out.flush();
        r
    }

    pub fn load_module(&mut self, key: &Rc<str>) -> Result<Rc<Module>, Flow> {
        match self.modules.get(key) {
            Some(ModState::Done(m)) => return Ok(m.clone()),
            Some(ModState::Loading) => {
                return Err(err("ImportError", format!("circular import of `{key}`")));
            }
            None => {}
        }
        let proto =
            self.program.module(key).cloned().ok_or_else(|| err("ImportError", format!("module `{key}` is not in this program")))?;
        self.modules.insert(key.clone(), ModState::Loading);
        let globals =
            Rc::new(Globals { names: RefCell::new(proto.globals.clone()), vals: RefCell::new(vec![Value::Undef; proto.globals.len()]) });
        let c = Rc::new(Closure { proto: proto.main.clone(), frees: Vec::new(), globals: globals.clone(), home: RefCell::new(None) });
        if let Err(e) = self.call(&Value::Func(c), &[]) {
            self.modules.remove(key);
            return Err(e);
        }
        let name = std::path::Path::new(&**key).file_stem().map_or("module".into(), |s| s.to_string_lossy().to_string());
        let m = Rc::new(Module { name: name.into(), kind: ModKind::File(globals) });
        self.modules.insert(key.clone(), ModState::Done(m.clone()));
        Ok(m)
    }

    fn builtin_module(&mut self, name: &Rc<str>) -> Result<Rc<Module>, Flow> {
        if let Some(m) = self.builtin_mods.get(name) {
            return Ok(m.clone());
        }
        let members = stdlib::module(name).ok_or_else(|| err("ImportError", format!("no built-in module `{name}`")))?;
        let m = Rc::new(Module { name: name.clone(), kind: ModKind::Native(members) });
        self.builtin_mods.insert(name.clone(), m.clone());
        Ok(m)
    }

    // call any callable from Rust (natives use this for callbacks)
    pub fn call(&mut self, f: &Value, args: &[Value]) -> R {
        self.call_kw(f, args, &[])
    }

    pub fn call_kw(&mut self, f: &Value, args: &[Value], kw: &[(Rc<str>, Value)]) -> R {
        if self.native_depth >= MAX_NATIVE_DEPTH {
            return Err(err("RecursionError", "too many nested callbacks"));
        }
        let depth = self.frames.len();
        let pos = self.stack.len();
        self.stack.push(f.clone());
        self.stack.extend(args.iter().cloned());
        self.stack.extend(kw.iter().map(|(_, v)| v.clone()));
        let names: Option<Rc<[Rc<str>]>> = if kw.is_empty() { None } else { Some(kw.iter().map(|(k, _)| k.clone()).collect()) };
        self.native_depth += 1;
        let r = match self.call_value(pos, args.len() + kw.len(), names) {
            Ok(()) if self.frames.len() > depth => self.run(depth),
            Ok(()) => Ok(self.stack.pop().unwrap()),
            Err(e) => Err(e),
        };
        self.native_depth -= 1;
        if r.is_err() {
            self.stack.truncate(pos);
        }
        r
    }

    fn run(&mut self, stop: usize) -> R {
        loop {
            match self.exec(stop) {
                Ok(v) => return Ok(v),
                Err(Flow::Throw(e)) => {
                    self.add_trace(&e);
                    if self.handlers.last().is_some_and(|h| h.frame >= stop) {
                        let h = self.handlers.pop().unwrap();
                        self.frames.truncate(h.frame + 1);
                        self.stack.truncate(h.stack);
                        self.frames[h.frame].ip = h.ip;
                        self.stack.push(e);
                        continue;
                    }
                    self.unwind_to(stop);
                    return Err(Flow::Throw(e));
                }
                Err(exit) => {
                    self.unwind_to(stop);
                    return Err(exit);
                }
            }
        }
    }

    fn unwind_to(&mut self, stop: usize) {
        if self.frames.len() > stop {
            let r = self.frames[stop].ret_to;
            self.frames.truncate(stop);
            self.stack.truncate(r);
        }
        while self.handlers.last().is_some_and(|h| h.frame >= stop) {
            self.handlers.pop();
        }
    }

    // record where an error happened (first time only)
    fn add_trace(&self, e: &Value) {
        let Value::Error(obj) = e else { return };
        let mut t = obj.trace.borrow_mut();
        if !t.is_empty() {
            return;
        }
        for f in self.frames.iter().rev() {
            let p = &f.closure.proto;
            let (line, col) = p.lines.get(f.ip.saturating_sub(1)).copied().unwrap_or((0, 0));
            t.push(TraceLine { func: p.name.clone(), file: p.file.clone(), line, col });
        }
    }

    fn exec(&mut self, stop: usize) -> R {
        let mut fi = self.frames.len() - 1;
        let mut closure = self.frames[fi].closure.clone();
        let mut ip = self.frames[fi].ip;
        let mut base = self.frames[fi].base;

        macro_rules! reload {
            () => {
                fi = self.frames.len() - 1;
                closure = self.frames[fi].closure.clone();
                ip = self.frames[fi].ip;
                base = self.frames[fi].base;
            };
        }
        macro_rules! tri {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.frames[fi].ip = ip;
                        return Err(e);
                    }
                }
            };
        }
        macro_rules! bail {
            ($e:expr) => {{
                self.frames[fi].ip = ip;
                return Err($e);
            }};
        }
        macro_rules! pop {
            () => {
                self.stack.pop().unwrap()
            };
        }
        macro_rules! name {
            ($k:expr) => {
                match &closure.proto.consts[$k as usize] {
                    Const::Str(s) => s.clone(),
                    _ => unreachable!(),
                }
            };
        }
        macro_rules! names {
            ($k:expr) => {
                match &closure.proto.consts[$k as usize] {
                    Const::Names(n) => Some(n.clone()),
                    _ => unreachable!(),
                }
            };
        }

        loop {
            let op = closure.proto.code[ip];
            ip += 1;
            match op {
                Op::Const(k) => {
                    let v = match &closure.proto.consts[k as usize] {
                        Const::Int(n) => Value::Int(*n),
                        Const::Float(x) => Value::Float(*x),
                        Const::Str(s) => Value::Str(s.clone()),
                        _ => unreachable!(),
                    };
                    self.stack.push(v);
                }
                Op::Nil => self.stack.push(Value::Nil),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Pop => {
                    self.stack.pop();
                }
                Op::Dup => self.stack.push(self.stack.last().unwrap().clone()),
                Op::Dup2 => {
                    let n = self.stack.len();
                    self.stack.push(self.stack[n - 2].clone());
                    self.stack.push(self.stack[n - 1].clone());
                }
                Op::Swap => {
                    let n = self.stack.len();
                    self.stack.swap(n - 1, n - 2);
                }
                Op::Rot3 => {
                    let v = self.stack.remove(self.stack.len() - 3);
                    self.stack.push(v);
                }
                Op::LoadLocal(s) => {
                    let v = match &self.stack[base + s as usize] {
                        Value::Cell(c) => c.borrow().clone(),
                        v => v.clone(),
                    };
                    if let Value::Undef = v {
                        let n = closure.proto.locals[s as usize].clone();
                        bail!(unassigned(&n));
                    }
                    self.stack.push(v);
                }
                Op::StoreLocal(s) => {
                    let v = pop!();
                    match &self.stack[base + s as usize] {
                        Value::Cell(c) => *c.borrow_mut() = v,
                        _ => self.stack[base + s as usize] = v,
                    }
                }
                Op::LoadFree(i) => {
                    let v = closure.frees[i as usize].borrow().clone();
                    if let Value::Undef = v {
                        bail!(unassigned(&closure.proto.free_names[i as usize]));
                    }
                    self.stack.push(v);
                }
                Op::StoreFree(i) => {
                    let v = pop!();
                    *closure.frees[i as usize].borrow_mut() = v;
                }
                Op::LoadGlobal(g) => {
                    let v = closure.globals.vals.borrow()[g as usize].clone();
                    if let Value::Undef = v {
                        let n = closure.globals.names.borrow()[g as usize].clone();
                        bail!(unassigned(&n));
                    }
                    self.stack.push(v);
                }
                Op::StoreGlobal(g) => {
                    let v = pop!();
                    closure.globals.vals.borrow_mut()[g as usize] = v;
                }
                Op::LoadBuiltin(b) => self.stack.push(Value::Native(&stdlib::BUILTINS[b as usize])),
                Op::GetProp(k) => {
                    let obj = pop!();
                    let v = tri!(self.get_prop(&obj, &name!(k)));
                    self.stack.push(v);
                }
                Op::SetProp(k) => {
                    let v = pop!();
                    let obj = pop!();
                    tri!(set_prop(&obj, &name!(k), v));
                }
                Op::GetIndex => {
                    let i = pop!();
                    let obj = pop!();
                    let v = tri!(ops::get_index(&obj, &i));
                    self.stack.push(v);
                }
                Op::SetIndex => {
                    let v = pop!();
                    let i = pop!();
                    let obj = pop!();
                    tri!(ops::set_index(&obj, &i, v));
                }
                Op::GetSlice => {
                    let hi = pop!();
                    let lo = pop!();
                    let obj = pop!();
                    let v = tri!(ops::get_slice(&obj, &lo, &hi));
                    self.stack.push(v);
                }
                Op::Add | Op::Sub | Op::Mul | Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Eq | Op::Ne => {
                    let b = pop!();
                    let a = pop!();
                    // int fast path for hot loops
                    let v = match (op, &a, &b) {
                        (Op::Lt, Value::Int(x), Value::Int(y)) => Value::Bool(x < y),
                        (Op::Le, Value::Int(x), Value::Int(y)) => Value::Bool(x <= y),
                        (Op::Gt, Value::Int(x), Value::Int(y)) => Value::Bool(x > y),
                        (Op::Ge, Value::Int(x), Value::Int(y)) => Value::Bool(x >= y),
                        (Op::Add, Value::Float(x), Value::Float(y)) => Value::Float(x + y),
                        (Op::Mul, Value::Float(x), Value::Float(y)) => Value::Float(x * y),
                        _ => tri!(ops::binary(op, &a, &b)),
                    };
                    self.stack.push(v);
                }
                Op::Div | Op::IntDiv | Op::Mod | Op::Pow | Op::In | Op::NotIn | Op::Approx => {
                    let b = pop!();
                    let a = pop!();
                    let v = tri!(ops::binary(op, &a, &b));
                    self.stack.push(v);
                }
                Op::Neg => {
                    let v = match pop!() {
                        Value::Int(n) => Value::Int(tri!(n.checked_neg().ok_or_else(|| err("OverflowError", "integer overflow")))),
                        Value::Float(x) => Value::Float(-x),
                        other => bail!(type_err(format!("cannot negate {}", other.kind_name()))),
                    };
                    self.stack.push(v);
                }
                Op::Not => {
                    let v = pop!();
                    self.stack.push(Value::Bool(!v.truthy()));
                }
                Op::Jump(t) => {
                    // loop back-edge: check time limit
                    if (t as usize) < ip
                        && let Some(d) = self.deadline
                        && std::time::Instant::now() > d
                    {
                        bail!(timeout());
                    }
                    ip = t as usize
                }
                Op::JumpIfFalse(t) => {
                    if !pop!().truthy() {
                        ip = t as usize;
                    }
                }
                Op::JumpIfFalseKeep(t) => {
                    if self.stack.last().unwrap().truthy() {
                        self.stack.pop();
                    } else {
                        ip = t as usize;
                    }
                }
                Op::JumpIfTrueKeep(t) => {
                    if self.stack.last().unwrap().truthy() {
                        ip = t as usize;
                    } else {
                        self.stack.pop();
                    }
                }
                Op::Call(argc) | Op::CallKw(argc, _) => {
                    let kw = if let Op::CallKw(_, k) = op { names!(k) } else { None };
                    self.frames[fi].ip = ip;
                    let pos = self.stack.len() - argc as usize - 1;
                    tri!(self.call_value(pos, argc as usize, kw));
                    reload!();
                }
                Op::Invoke(k, argc) | Op::InvokeKw(k, argc, _) => {
                    let kw = if let Op::InvokeKw(_, _, n) = op { names!(n) } else { None };
                    self.frames[fi].ip = ip;
                    tri!(self.invoke(&name!(k), argc as usize, kw));
                    reload!();
                }
                Op::GetSuper(k) => {
                    let me = pop!();
                    let n = name!(k);
                    let home = closure.home.borrow().clone();
                    let m = home.as_ref().and_then(|c| c.methods.borrow().get(&n).cloned());
                    match m {
                        Some(m) => self.stack.push(Value::Bound(Rc::new((me, m)))),
                        None => bail!(err("AttributeError", format!("superclass has no method `{n}`"))),
                    }
                }
                Op::Return => {
                    let mut result = pop!();
                    let frame = self.frames.pop().unwrap();
                    if let Some(s) = frame.init_self {
                        result = s;
                    }
                    while self.handlers.last().is_some_and(|h| h.frame >= self.frames.len()) {
                        self.handlers.pop();
                    }
                    self.stack.truncate(frame.ret_to);
                    if self.frames.len() == stop {
                        return Ok(result);
                    }
                    self.stack.push(result);
                    reload!();
                }
                Op::Closure(k) => {
                    let Const::Func(proto) = &closure.proto.consts[k as usize] else { unreachable!() };
                    let frees = proto
                        .captures
                        .iter()
                        .map(|c| match c {
                            Capture::Local(s) => match &self.stack[base + *s as usize] {
                                Value::Cell(c) => c.clone(),
                                v => Rc::new(RefCell::new(v.clone())),
                            },
                            Capture::Free(i) => closure.frees[*i as usize].clone(),
                        })
                        .collect();
                    let c = Closure { proto: proto.clone(), frees, globals: closure.globals.clone(), home: RefCell::new(None) };
                    self.stack.push(Value::Func(Rc::new(c)));
                }
                Op::Class(k, has_sup) => {
                    let sup = if has_sup {
                        match pop!() {
                            Value::Class(c) => Some(c),
                            other => bail!(type_err(format!("can only inherit from a class, not {}", other.kind_name()))),
                        }
                    } else {
                        None
                    };
                    let methods = sup.as_ref().map(|s| s.methods.borrow().clone()).unwrap_or_default();
                    let c = Class { name: name!(k), methods: RefCell::new(methods), sup };
                    self.stack.push(Value::Class(Rc::new(c)));
                }
                Op::Method(k) => {
                    let Value::Func(m) = pop!() else { unreachable!() };
                    let Value::Class(c) = self.stack.last().unwrap() else { unreachable!() };
                    *m.home.borrow_mut() = c.sup.clone();
                    c.methods.borrow_mut().insert(name!(k), m);
                }
                Op::MakeList(n) => {
                    let items = self.stack.split_off(self.stack.len() - n as usize);
                    self.stack.push(Value::list(items));
                }
                Op::MakeMap(n) => {
                    let items = self.stack.split_off(self.stack.len() - 2 * n as usize);
                    let mut m = IndexMap::with_capacity(n as usize);
                    let mut it = items.into_iter();
                    while let (Some(k), Some(v)) = (it.next(), it.next()) {
                        m.insert(tri!(Key::from(&k)), v);
                    }
                    self.stack.push(Value::map(m));
                }
                Op::MakeRange(incl) => {
                    let hi = pop!();
                    let lo = pop!();
                    let a = tri!(lo.int("range start"));
                    let b = tri!(hi.int("range end"));
                    let b = if incl { tri!(b.checked_add(1).ok_or_else(|| err("OverflowError", "range end too big"))) } else { b };
                    self.stack.push(Value::Range(a, b));
                }
                Op::Unpack(n) => {
                    let v = pop!();
                    let items: Vec<Value> = match &v {
                        Value::List(l) => l.borrow().clone(),
                        other => bail!(type_err(format!("cannot unpack {} into {n} names", other.kind_name()))),
                    };
                    if items.len() != n as usize {
                        bail!(value_err(format!("expected {n} values to unpack, got {}", items.len())));
                    }
                    self.stack.extend(items);
                }
                Op::IterInit => {
                    let v = pop!();
                    let it = tri!(ops::iter_init(&v));
                    self.stack.push(it);
                }
                Op::IterNext(t) => {
                    let it = pop!();
                    match ops::iter_next(&it) {
                        Some(x) => self.stack.push(x),
                        None => ip = t as usize,
                    }
                }
                Op::TryBegin(t) => self.handlers.push(Handler { frame: fi, stack: self.stack.len(), ip: t as usize }),
                Op::TryEnd => {
                    self.handlers.pop();
                }
                Op::Throw => {
                    let v = pop!();
                    let e = match v {
                        Value::Error(_) => v,
                        Value::Str(s) => match err("Error", &*s) {
                            Flow::Throw(e) => e,
                            _ => unreachable!(),
                        },
                        other => {
                            let msg = tri!(self.display(&other, false));
                            match err("Error", msg) {
                                Flow::Throw(e) => e,
                                _ => unreachable!(),
                            }
                        }
                    };
                    bail!(Flow::Throw(e));
                }
                Op::Format(k) => {
                    let v = pop!();
                    self.frames[fi].ip = ip;
                    let s = if k == NO_SPEC { tri!(self.display(&v, false)) } else { tri!(stdlib::fmt::format_spec(self, &v, &name!(k))) };
                    self.stack.push(Value::str(s));
                }
                Op::Concat(n) => {
                    let parts = self.stack.split_off(self.stack.len() - n as usize);
                    let mut s = String::new();
                    for p in &parts {
                        if let Value::Str(x) = p {
                            s.push_str(x);
                        }
                    }
                    self.stack.push(Value::str(s));
                }
                Op::DefaultArg(s, t) => {
                    let set = match &self.stack[base + s as usize] {
                        Value::Cell(c) => !matches!(*c.borrow(), Value::Undef),
                        v => !matches!(v, Value::Undef),
                    };
                    if set {
                        ip = t as usize;
                    }
                }
                Op::Import(k) => {
                    self.frames[fi].ip = ip;
                    let m = tri!(self.load_module(&name!(k)));
                    self.stack.push(Value::Module(m));
                }
                Op::ImportBuiltin(k) => {
                    let m = tri!(self.builtin_module(&name!(k)));
                    self.stack.push(Value::Module(m));
                }
                Op::ReplPrint => {
                    let v = pop!();
                    if !matches!(v, Value::Nil) {
                        self.frames[fi].ip = ip;
                        let s = tri!(self.display(&v, true));
                        let _ = writeln!(self.out, "{s}");
                    }
                }
                Op::RegisterTest(kind, k) => {
                    let gens = match pop!() {
                        Value::List(l) => l.borrow().clone(),
                        _ => unreachable!(),
                    };
                    let opts = pop!();
                    let func = pop!();
                    let line = closure.proto.lines.get(ip - 1).map_or(0, |l| l.0);
                    self.tests.push(TestDef { kind, name: name!(k), func, opts, gens, file: closure.proto.file.clone(), line });
                }
                Op::Expect(k) => {
                    let v = pop!();
                    if !v.truthy() {
                        self.frames[fi].ip = ip;
                        let shown = tri!(self.display(&v, true));
                        bail!(err("ExpectFailed", format!("expected `{}`\n  got: {shown}", name!(k))));
                    }
                }
                Op::ExpectCmp(c, k) => {
                    let r = pop!();
                    let l = pop!();
                    if !tri!(ops::binary(c.op(), &l, &r)).truthy() {
                        self.frames[fi].ip = ip;
                        let (ls, rs) = (tri!(self.display(&l, true)), tri!(self.display(&r, true)));
                        bail!(err("ExpectFailed", format!("expected `{}`\n  left:  {ls}\n  right: {rs}", name!(k))));
                    }
                }
                Op::ExpectApprox(k) => {
                    let tol = pop!();
                    let r = pop!();
                    let l = pop!();
                    let t = tri!(tol.num("within"));
                    if !tri!(approx(&l, &r, 0.0, t)) {
                        self.frames[fi].ip = ip;
                        let (ls, rs) = (tri!(self.display(&l, true)), tri!(self.display(&r, true)));
                        bail!(err(
                            "ExpectFailed",
                            format!("expected `{}` within {}\n  left:  {ls}\n  right: {rs}", name!(k), fmt_float(t))
                        ));
                    }
                }
                Op::Report => {
                    let v = pop!();
                    let label = pop!();
                    self.frames[fi].ip = ip;
                    let label = tri!(self.display(&label, false));
                    match &mut self.test_ctx {
                        Some(ctx) => ctx.reports.push((label, v)),
                        None => {
                            let shown = tri!(self.display(&v, false));
                            let _ = writeln!(self.out, "{label}: {shown}");
                        }
                    }
                }
            }
        }
    }

    // pop args above `pos` into Args; stack ends at pos
    fn take_args(&mut self, pos: usize, kw: Option<Rc<[Rc<str>]>>) -> Args {
        let mut pos_vals = self.stack.split_off(pos + 1);
        self.stack.truncate(pos);
        let kw = match kw {
            Some(names) => {
                let kv = pos_vals.split_off(pos_vals.len() - names.len());
                names.iter().cloned().zip(kv).collect()
            }
            None => Vec::new(),
        };
        Args { pos: pos_vals, kw }
    }

    fn call_value(&mut self, pos: usize, argc: usize, kw: Option<Rc<[Rc<str>]>>) -> Result<(), Flow> {
        let callee = self.stack[pos].clone();
        match callee {
            Value::Func(c) => self.push_frame(c, pos + 1, pos, argc, kw, None),
            Value::Bound(b) => {
                self.stack[pos] = b.0.clone();
                self.push_frame(b.1.clone(), pos, pos, argc + 1, kw, None)
            }
            Value::Class(cls) => {
                let inst = Value::Instance(Rc::new(Instance { class: cls.clone(), fields: RefCell::new(IndexMap::new()) }));
                let init = cls.methods.borrow().get("init").cloned();
                match init {
                    Some(m) => {
                        self.stack[pos] = inst.clone();
                        self.push_frame(m, pos, pos, argc + 1, kw, Some(inst))
                    }
                    None if argc > 0 => Err(type_err(format!("{}() takes no arguments (it has no `init`)", cls.name))),
                    None => {
                        self.stack.truncate(pos);
                        self.stack.push(inst);
                        Ok(())
                    }
                }
            }
            Value::Native(n) => {
                let args = self.take_args(pos, kw);
                let r = (n.f)(self, args).map_err(|e| with_context(e, n.name))?;
                self.stack.push(r);
                Ok(())
            }
            Value::BoundNative(b) => {
                let args = self.take_args(pos, kw);
                let r = stdlib::call_method(self, &b.0, &b.1, args).map_err(|e| with_context(e, &b.1))?;
                self.stack.push(r);
                Ok(())
            }
            other => Err(type_err(format!("{} is not callable", other.kind_name()))),
        }
    }

    fn push_frame(
        &mut self,
        c: Rc<Closure>,
        base: usize,
        ret_to: usize,
        argc: usize,
        kw: Option<Rc<[Rc<str>]>>,
        init_self: Option<Value>,
    ) -> Result<(), Flow> {
        let p = c.proto.clone();
        let nkw = kw.as_ref().map_or(0, |k| k.len());
        let npos = argc - nkw;
        let is_method = base == ret_to;
        if npos > p.params.len() {
            let shift = is_method as usize;
            return Err(type_err(format!("{}() takes {} argument(s) but got {}", p.name, p.params.len() - shift, npos - shift)));
        }
        let kwvals = if nkw > 0 { self.stack.split_off(base + npos) } else { Vec::new() };
        self.stack.resize(base + p.nlocals as usize, Value::Undef);
        if let Some(names) = kw {
            for (name, v) in names.iter().zip(kwvals) {
                let Some(i) = p.params.iter().position(|n| n == name) else {
                    return Err(type_err(format!("{}() has no parameter `{name}`", p.name)));
                };
                if !matches!(self.stack[base + i], Value::Undef) {
                    return Err(type_err(format!("{}() got `{name}` twice", p.name)));
                }
                self.stack[base + i] = v;
            }
        }
        for i in 0..p.required as usize {
            if matches!(self.stack[base + i], Value::Undef) {
                return Err(type_err(format!("{}() missing argument `{}`", p.name, p.params[i])));
            }
        }
        for &s in &p.cells {
            let v = std::mem::replace(&mut self.stack[base + s as usize], Value::Undef);
            self.stack[base + s as usize] = Value::Cell(Rc::new(RefCell::new(v)));
        }
        if self.frames.len() >= MAX_FRAMES {
            return Err(err("RecursionError", "too deep recursion (over 20000 calls)"));
        }
        if self.deadline.is_some_and(|d| std::time::Instant::now() > d) {
            return Err(timeout());
        }
        self.frames.push(Frame { closure: c, ip: 0, base, ret_to, init_self });
        Ok(())
    }

    // obj.name(args) without making a bound method
    fn invoke(&mut self, name: &Rc<str>, argc: usize, kw: Option<Rc<[Rc<str>]>>) -> Result<(), Flow> {
        let pos = self.stack.len() - argc - 1;
        let recv = self.stack[pos].clone();
        match &recv {
            Value::Instance(inst) => {
                let field = inst.fields.borrow().get(&**name).cloned();
                if let Some(f) = field {
                    self.stack[pos] = f;
                    return self.call_value(pos, argc, kw);
                }
                let m = inst.class.methods.borrow().get(&**name).cloned();
                match m {
                    Some(m) => self.push_frame(m, pos, pos, argc + 1, kw, None),
                    None => Err(no_attr(&recv, name)),
                }
            }
            Value::Module(_) | Value::Class(_) => {
                self.stack[pos] = self.get_prop(&recv, name)?;
                self.call_value(pos, argc, kw)
            }
            Value::Map(m) if !stdlib::has_method(&recv, name) => {
                let v = m.borrow().get(&Key::Str(name.clone())).cloned();
                match v {
                    Some(v) => {
                        self.stack[pos] = v;
                        self.call_value(pos, argc, kw)
                    }
                    None => Err(no_attr(&recv, name)),
                }
            }
            _ => {
                if !stdlib::has_method(&recv, name) {
                    return Err(no_attr(&recv, name));
                }
                let args = self.take_args(pos, kw);
                let r = stdlib::call_method(self, &recv, name, args).map_err(|e| with_context(e, name))?;
                self.stack.push(r);
                Ok(())
            }
        }
    }

    pub fn get_prop(&mut self, obj: &Value, name: &Rc<str>) -> R {
        match obj {
            Value::Instance(inst) => {
                if let Some(v) = inst.fields.borrow().get(&**name) {
                    return Ok(v.clone());
                }
                match inst.class.methods.borrow().get(&**name) {
                    Some(m) => Ok(Value::Bound(Rc::new((obj.clone(), m.clone())))),
                    None => Err(no_attr(obj, name)),
                }
            }
            Value::Module(m) => module_get(m, name),
            Value::Class(c) => {
                if &**name == "name" {
                    return Ok(Value::Str(c.name.clone()));
                }
                c.methods.borrow().get(&**name).map(|m| Value::Func(m.clone())).ok_or_else(|| no_attr(obj, name))
            }
            Value::Map(m) => {
                if let Some(v) = m.borrow().get(&Key::Str(name.clone())) {
                    return Ok(v.clone());
                }
                if stdlib::has_method(obj, name) {
                    return Ok(Value::BoundNative(Rc::new((obj.clone(), name.clone()))));
                }
                Err(err("KeyError", format!("map has no key \"{name}\"")))
            }
            Value::Error(e) => match &**name {
                "message" => Ok(Value::Str(e.message.clone())),
                "kind" => Ok(Value::Str(e.kind.clone())),
                "trace" => Ok(Value::str(format_trace(&e.trace.borrow()))),
                _ => Err(no_attr(obj, name)),
            },
            Value::Object(o) => match o.get(name) {
                Some(v) => Ok(v),
                None if o.methods().contains(&&**name) => Ok(Value::BoundNative(Rc::new((obj.clone(), name.clone())))),
                None => Err(no_attr(obj, name)),
            },
            _ if stdlib::has_method(obj, name) => Ok(Value::BoundNative(Rc::new((obj.clone(), name.clone())))),
            _ => Err(no_attr(obj, name)),
        }
    }

    // text form of a value; repr quotes strings
    pub fn display(&mut self, v: &Value, repr: bool) -> Result<String, Flow> {
        let mut s = String::new();
        self.write_value(&mut s, v, repr, 0)?;
        Ok(s)
    }

    fn write_value(&mut self, out: &mut String, v: &Value, repr: bool, depth: usize) -> Result<(), Flow> {
        use std::fmt::Write as _;
        if depth > 40 {
            out.push_str("...");
            return Ok(());
        }
        match v {
            Value::Undef => out.push_str("<undefined>"),
            Value::Nil => out.push_str("nil"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Value::Float(x) => out.push_str(&fmt_float(*x)),
            Value::Str(s) if repr => {
                let _ = write!(out, "{:?}", &**s);
            }
            Value::Str(s) => out.push_str(s),
            Value::List(l) => {
                let items = l.borrow().clone();
                out.push('[');
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.write_value(out, x, true, depth + 1)?;
                }
                out.push(']');
            }
            Value::Map(m) => {
                let items: Vec<(Key, Value)> = m.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                out.push('{');
                for (i, (k, x)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.write_value(out, &k.value(), true, depth + 1)?;
                    out.push_str(": ");
                    self.write_value(out, x, true, depth + 1)?;
                }
                out.push('}');
            }
            Value::Range(a, b) => {
                let _ = write!(out, "{a}..{b}");
            }
            Value::Func(c) => {
                let _ = write!(out, "<fn {}>", c.proto.name);
            }
            Value::Native(n) => {
                let _ = write!(out, "<built-in fn {}>", n.name);
            }
            Value::BoundNative(b) => {
                let _ = write!(out, "<method {}.{}>", b.0.type_name(), b.1);
            }
            Value::Bound(b) => {
                let _ = write!(out, "<method {}.{}>", b.0.kind_name(), b.1.proto.name);
            }
            Value::Class(c) => {
                let _ = write!(out, "<class {}>", c.name);
            }
            Value::Module(m) => {
                let _ = write!(out, "<module {}>", m.name);
            }
            Value::Error(e) => {
                let _ = write!(out, "{}: {}", e.kind, e.message);
            }
            Value::Instance(inst) => {
                let to_str = inst.class.methods.borrow().get("to_str").cloned();
                if let Some(m) = to_str {
                    let r = self.call(&Value::Bound(Rc::new((v.clone(), m))), &[])?;
                    match r {
                        Value::Str(s) => out.push_str(&s),
                        other => {
                            return Err(type_err(format!("to_str() must return str, got {}", other.kind_name())));
                        }
                    }
                    return Ok(());
                }
                let fields: Vec<(Rc<str>, Value)> = inst.fields.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let _ = write!(out, "{}(", inst.class.name);
                for (i, (k, x)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "{k}=");
                    self.write_value(out, x, true, depth + 1)?;
                }
                out.push(')');
            }
            Value::Cell(c) => {
                let inner = c.borrow().clone();
                self.write_value(out, &inner, repr, depth)?;
            }
            Value::Iter(_) => out.push_str("<iterator>"),
            Value::Object(o) => out.push_str(&o.display()),
        }
        Ok(())
    }
}

fn timeout() -> Flow {
    err("TimeoutError", "time limit reached")
}

fn unassigned(name: &str) -> Flow {
    err("NameError", format!("`{name}` is used before it gets a value"))
}

fn no_attr(obj: &Value, name: &str) -> Flow {
    match obj {
        Value::Instance(i) => err("AttributeError", format!("{} has no field or method `{name}`", i.class.name)),
        Value::Module(m) => err("AttributeError", format!("module `{}` has no `{name}`", m.name)),
        Value::Class(c) => err("AttributeError", format!("class {} has no method `{name}`", c.name)),
        Value::Map(_) => err("AttributeError", format!("map has no key or method `{name}`")),
        other => err("AttributeError", format!("{} has no method `{name}`", other.type_name())),
    }
}

// prefix native error messages with the function name
fn with_context(e: Flow, fname: &str) -> Flow {
    match e {
        Flow::Throw(Value::Error(obj))
            if obj.trace.borrow().is_empty()
                && (&*obj.kind == "TypeError" || &*obj.kind == "ValueError")
                && !obj.message.starts_with(fname) =>
        {
            Flow::Throw(Value::Error(Rc::new(ErrorObj {
                kind: obj.kind.clone(),
                message: format!("{fname}(): {}", obj.message).into(),
                trace: RefCell::new(Vec::new()),
            })))
        }
        other => other,
    }
}

fn set_prop(obj: &Value, name: &Rc<str>, v: Value) -> Result<(), Flow> {
    match obj {
        Value::Instance(i) => {
            i.fields.borrow_mut().insert(name.clone(), v);
            Ok(())
        }
        Value::Map(m) => {
            m.borrow_mut().insert(Key::Str(name.clone()), v);
            Ok(())
        }
        other => Err(type_err(format!("cannot set `{name}` on {}", other.kind_name()))),
    }
}

pub fn module_get(m: &Module, name: &str) -> R {
    let v = match &m.kind {
        ModKind::Native(members) => members.get(name).cloned(),
        ModKind::File(g) => {
            let idx = g.names.borrow().iter().position(|n| &**n == name);
            idx.map(|i| g.vals.borrow()[i].clone()).filter(|v| !matches!(v, Value::Undef))
        }
    };
    v.ok_or_else(|| err("AttributeError", format!("module `{}` has no `{name}`", m.name)))
}

pub fn format_trace(t: &[TraceLine]) -> String {
    let mut s = String::new();
    for l in t {
        s.push_str(&format!("  at {} ({}:{}:{})\n", l.func, l.file, l.line, l.col));
    }
    s
}
