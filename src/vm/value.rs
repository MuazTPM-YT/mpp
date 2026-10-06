use super::Vm;
use crate::compile::chunk::FuncProto;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt;
use std::rc::Rc;

pub type R = Result<Value, Flow>;
pub type NativeFn = fn(&mut Vm, Args) -> R;
pub type ListRef = Rc<RefCell<Vec<Value>>>;
pub type MapRef = Rc<RefCell<IndexMap<Key, Value>>>;

// non-local control: thrown error, or exit()
#[derive(Debug, Clone)]
pub enum Flow {
    Throw(Value),
    Exit(i32),
}

#[derive(Debug, Clone)]
pub struct TraceLine {
    pub func: Rc<str>,
    pub file: Rc<str>,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug)]
pub struct ErrorObj {
    pub kind: Rc<str>,
    pub message: Rc<str>,
    pub trace: RefCell<Vec<TraceLine>>,
}

// map keys: only hashable values
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Nil,
    Bool(bool),
    Int(i64),
    Str(Rc<str>),
}

pub struct Globals {
    pub names: RefCell<Vec<Rc<str>>>,
    pub vals: RefCell<Vec<Value>>,
}

pub struct Closure {
    pub proto: Rc<FuncProto>,
    pub frees: Vec<Rc<RefCell<Value>>>,
    pub globals: Rc<Globals>,
    // superclass, for `super.x` inside methods
    pub home: RefCell<Option<Rc<Class>>>,
}

pub struct Class {
    pub name: Rc<str>,
    pub methods: RefCell<IndexMap<Rc<str>, Rc<Closure>>>,
    pub sup: Option<Rc<Class>>,
}

pub struct Instance {
    pub class: Rc<Class>,
    pub fields: RefCell<IndexMap<Rc<str>, Value>>,
}

pub enum ModKind {
    Native(IndexMap<Rc<str>, Value>),
    File(Rc<Globals>),
}

pub struct Module {
    pub name: Rc<str>,
    pub kind: ModKind,
}

pub struct Native {
    pub name: &'static str,
    pub f: NativeFn,
}

pub enum IterState {
    Range(i64, i64),
    List(ListRef, usize),
    Items(Vec<Value>, usize),
}

#[derive(Clone)]
pub enum Value {
    // unassigned slot; never visible to user code
    Undef,
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    List(ListRef),
    Map(MapRef),
    Range(i64, i64),
    Func(Rc<Closure>),
    Native(&'static Native),
    BoundNative(Rc<(Value, Rc<str>)>),
    Bound(Rc<(Value, Rc<Closure>)>),
    Class(Rc<Class>),
    Instance(Rc<Instance>),
    Module(Rc<Module>),
    Error(Rc<ErrorObj>),
    // captured local, boxed; internal
    Cell(Rc<RefCell<Value>>),
    Iter(Rc<RefCell<IterState>>),
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => write!(f, "{}", fmt_float(*x)),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Nil => write!(f, "nil"),
            other => write!(f, "<{}>", other.type_name()),
        }
    }
}

pub fn err(kind: &str, msg: impl Into<String>) -> Flow {
    let msg: String = msg.into();
    Flow::Throw(Value::Error(Rc::new(ErrorObj { kind: kind.into(), message: msg.into(), trace: RefCell::new(Vec::new()) })))
}

pub fn type_err(msg: impl Into<String>) -> Flow {
    err("TypeError", msg)
}

pub fn value_err(msg: impl Into<String>) -> Flow {
    err("ValueError", msg)
}

// python-like float text: 1.0, 0.1, 1e-7, inf, nan
pub fn fmt_float(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let a = x.abs();
    if a != 0.0 && !(1e-4..1e16).contains(&a) {
        return py_exp(format!("{x:e}"), false);
    }
    let s = format!("{x}");
    if s.contains('.') { s } else { format!("{s}.0") }
}

// rust "1.5e-7" to python "1.5e-07"
pub fn py_exp(s: String, upper: bool) -> String {
    let Some(pos) = s.find('e') else { return s };
    let (m, e) = s.split_at(pos);
    let e = &e[1..];
    let (sign, digits) = if let Some(d) = e.strip_prefix('-') { ('-', d) } else { ('+', e) };
    let out = format!("{m}e{sign}{digits:0>2}");
    if upper { out.to_uppercase() } else { out }
}

impl Value {
    pub fn str(s: impl AsRef<str>) -> Value {
        Value::Str(s.as_ref().into())
    }

    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(v)))
    }

    pub fn map(m: IndexMap<Key, Value>) -> Value {
        Value::Map(Rc::new(RefCell::new(m)))
    }

    // map with string keys, for result records
    pub fn record<const N: usize>(fields: [(&str, Value); N]) -> Value {
        Value::map(fields.into_iter().map(|(k, v)| (Key::Str(k.into()), v)).collect())
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Undef => "undefined",
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Range(..) => "range",
            Value::Func(_) | Value::Native(_) | Value::BoundNative(_) | Value::Bound(_) => "function",
            Value::Class(_) => "class",
            Value::Instance(_) => "instance",
            Value::Module(_) => "module",
            Value::Error(_) => "error",
            Value::Cell(_) => "cell",
            Value::Iter(_) => "iterator",
        }
    }

    // name shown in messages: class name for instances
    pub fn kind_name(&self) -> String {
        match self {
            Value::Instance(i) => i.class.name.to_string(),
            other => other.type_name().to_string(),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Nil | Value::Undef => false,
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(x) => *x != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Map(m) => !m.borrow().is_empty(),
            Value::Range(a, b) => b > a,
            _ => true,
        }
    }

    pub fn num(&self, what: &str) -> Result<f64, Flow> {
        match self {
            Value::Int(n) => Ok(*n as f64),
            Value::Float(x) => Ok(*x),
            Value::Bool(b) => Ok(*b as i64 as f64),
            other => Err(type_err(format!("{what} must be a number, got {}", other.kind_name()))),
        }
    }

    pub fn int(&self, what: &str) -> Result<i64, Flow> {
        match self {
            Value::Int(n) => Ok(*n),
            Value::Bool(b) => Ok(*b as i64),
            Value::Float(x) if x.fract() == 0.0 && x.abs() < 9.2e18 => Ok(*x as i64),
            other => Err(type_err(format!("{what} must be a whole number, got {}", other.kind_name()))),
        }
    }

    pub fn as_str(&self, what: &str) -> Result<&Rc<str>, Flow> {
        match self {
            Value::Str(s) => Ok(s),
            other => Err(type_err(format!("{what} must be a string, got {}", other.kind_name()))),
        }
    }

    pub fn as_list(&self, what: &str) -> Result<&ListRef, Flow> {
        match self {
            Value::List(l) => Ok(l),
            other => Err(type_err(format!("{what} must be a list, got {}", other.kind_name()))),
        }
    }

    pub fn is_callable(&self) -> bool {
        matches!(self, Value::Func(_) | Value::Native(_) | Value::BoundNative(_) | Value::Bound(_) | Value::Class(_))
    }
}

impl Key {
    pub fn from(v: &Value) -> Result<Key, Flow> {
        Ok(match v {
            Value::Nil => Key::Nil,
            Value::Bool(b) => Key::Bool(*b),
            Value::Int(n) => Key::Int(*n),
            Value::Float(x) if x.fract() == 0.0 && x.abs() < 9.2e18 => Key::Int(*x as i64),
            Value::Str(s) => Key::Str(s.clone()),
            other => {
                return Err(type_err(format!("{} cannot be a map key (use str, int, bool or nil)", other.kind_name())));
            }
        })
    }

    pub fn value(&self) -> Value {
        match self {
            Key::Nil => Value::Nil,
            Key::Bool(b) => Value::Bool(*b),
            Key::Int(n) => Value::Int(*n),
            Key::Str(s) => Value::Str(s.clone()),
        }
    }
}

// deep equality; numbers compare across int/float
pub fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => (*x as f64) == *y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::List(x), Value::List(y)) => {
            Rc::ptr_eq(x, y) || {
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| equal(a, b))
            }
        }
        (Value::Map(x), Value::Map(y)) => {
            Rc::ptr_eq(x, y) || {
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| equal(v, w)))
            }
        }
        (Value::Range(a1, b1), Value::Range(a2, b2)) => a1 == a2 && b1 == b2,
        (Value::Func(x), Value::Func(y)) => Rc::ptr_eq(x, y),
        (Value::Native(x), Value::Native(y)) => std::ptr::eq(*x, *y),
        (Value::Class(x), Value::Class(y)) => Rc::ptr_eq(x, y),
        (Value::Instance(x), Value::Instance(y)) => Rc::ptr_eq(x, y),
        (Value::Module(x), Value::Module(y)) => Rc::ptr_eq(x, y),
        (Value::Error(x), Value::Error(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

// ordering for <, sort, min, max
pub fn compare(a: &Value, b: &Value) -> Result<Ordering, Flow> {
    let bad = || type_err(format!("cannot compare {} with {}", a.kind_name(), b.kind_name()));
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Int(_) | Value::Float(_) | Value::Bool(_), Value::Int(_) | Value::Float(_) | Value::Bool(_)) => {
            let (x, y) = (a.num("")?, b.num("")?);
            x.partial_cmp(&y).ok_or_else(|| value_err("cannot compare nan"))
        }
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        (Value::List(x), Value::List(y)) => {
            let (x, y) = (x.borrow(), y.borrow());
            for (p, q) in x.iter().zip(y.iter()) {
                match compare(p, q)? {
                    Ordering::Equal => {}
                    o => return Ok(o),
                }
            }
            Ok(x.len().cmp(&y.len()))
        }
        _ => Err(bad()),
    }
}

// a ~= b: rel 1e-6 or abs 1e-12, like pytest.approx
pub fn approx(a: &Value, b: &Value, rel: f64, abs: f64) -> Result<bool, Flow> {
    match (a, b) {
        (Value::List(x), Value::List(y)) => {
            let (x, y) = (x.borrow(), y.borrow());
            if x.len() != y.len() {
                return Ok(false);
            }
            for (p, q) in x.iter().zip(y.iter()) {
                if !approx(p, q, rel, abs)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => {
            let (x, y) = (a.num("left side of ~=")?, b.num("right side of ~=")?);
            if x == y {
                return Ok(true);
            }
            if x.is_nan() || y.is_nan() {
                return Ok(x.is_nan() && y.is_nan());
            }
            Ok((x - y).abs() <= (rel * x.abs().max(y.abs())).max(abs))
        }
    }
}

// arguments to a native function
pub struct Args {
    pub pos: Vec<Value>,
    pub kw: Vec<(Rc<str>, Value)>,
}

impl Args {
    pub fn new(pos: Vec<Value>) -> Args {
        Args { pos, kw: Vec::new() }
    }

    // match positional + keyword args to parameter names
    pub fn bind<const N: usize>(self, names: [&str; N]) -> Result<[Option<Value>; N], Flow> {
        if self.pos.len() > N {
            return Err(type_err(format!("takes at most {N} argument(s) ({}), got {}", names.join(", "), self.pos.len())));
        }
        let mut out: [Option<Value>; N] = std::array::from_fn(|_| None);
        for (i, v) in self.pos.into_iter().enumerate() {
            out[i] = Some(v);
        }
        for (k, v) in self.kw {
            match names.iter().position(|n| **n == *k) {
                Some(i) if out[i].is_some() => {
                    return Err(type_err(format!("argument `{k}` given twice")));
                }
                Some(i) => out[i] = Some(v),
                None => {
                    return Err(type_err(format!("unknown argument `{k}` (takes {})", names.join(", "))));
                }
            }
        }
        Ok(out)
    }

    pub fn no_kw(&self) -> Result<(), Flow> {
        match self.kw.first() {
            Some((k, _)) => Err(type_err(format!("unknown argument `{k}`"))),
            None => Ok(()),
        }
    }
}

// required argument
pub fn need(v: Option<Value>, name: &str) -> Result<Value, Flow> {
    v.ok_or_else(|| type_err(format!("missing argument `{name}`")))
}

// optional argument, nil counts as missing
pub fn opt(v: Option<Value>) -> Option<Value> {
    v.filter(|v| !matches!(v, Value::Nil))
}
