use serde::{Deserialize, Serialize};
use std::rc::Rc;

pub const NO_SPEC: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Const(u32),
    Nil,
    True,
    False,
    Pop,
    Dup,
    Dup2,
    Swap,
    Rot3,
    LoadLocal(u16),
    StoreLocal(u16),
    LoadFree(u16),
    StoreFree(u16),
    LoadGlobal(u32),
    StoreGlobal(u32),
    LoadBuiltin(u16),
    GetProp(u32),
    SetProp(u32),
    GetIndex,
    SetIndex,
    GetSlice,
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
    Neg,
    Not,
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfFalseKeep(u32),
    JumpIfTrueKeep(u32),
    Call(u8),
    // argc, const index of keyword names (last args are keywords)
    CallKw(u8, u32),
    Invoke(u32, u8),
    InvokeKw(u32, u8, u32),
    GetSuper(u32),
    Return,
    Closure(u32),
    Class(u32, bool),
    Method(u32),
    MakeList(u32),
    MakeMap(u32),
    MakeRange(bool),
    Unpack(u32),
    IterInit,
    IterNext(u32),
    TryBegin(u32),
    TryEnd,
    Throw,
    // const index of format spec, or NO_SPEC
    Format(u32),
    Concat(u32),
    // skip default-value code when param slot already has a value
    DefaultArg(u16, u32),
    Import(u32),
    ImportBuiltin(u32),
    ReplPrint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Const {
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Func(Rc<FuncProto>),
    Names(Rc<[Rc<str>]>),
}

// where a closure's captured cell comes from
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Capture {
    Local(u16),
    Free(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuncProto {
    pub name: Rc<str>,
    pub file: Rc<str>,
    pub params: Vec<Rc<str>>,
    // params before the first default
    pub required: u16,
    pub nlocals: u16,
    // slot names, for error messages
    pub locals: Vec<Rc<str>>,
    pub cells: Vec<u16>,
    pub captures: Vec<Capture>,
    pub free_names: Vec<Rc<str>>,
    pub code: Vec<Op>,
    pub consts: Vec<Const>,
    // (line, col) per op
    pub lines: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleProto {
    pub key: Rc<str>,
    pub globals: Vec<Rc<str>>,
    pub main: Rc<FuncProto>,
}

// everything needed to run: entry module + all imported modules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub entry: Rc<str>,
    pub modules: Vec<Rc<ModuleProto>>,
}

const MAGIC: &[u8; 4] = b"MPPC";

impl Program {
    pub fn module(&self, key: &str) -> Option<&Rc<ModuleProto>> {
        self.modules.iter().find(|m| &*m.key == key)
    }

    // bytes: magic, version string, postcard body
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let ver = env!("CARGO_PKG_VERSION").as_bytes();
        out.push(ver.len() as u8);
        out.extend_from_slice(ver);
        out.extend(postcard::to_allocvec(self).expect("program serializes"));
        out
    }

    pub fn is_bytecode(bytes: &[u8]) -> bool {
        bytes.starts_with(MAGIC)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Program, String> {
        if !Self::is_bytecode(bytes) || bytes.len() < 5 {
            return Err("not a Muaz++ bytecode file".into());
        }
        let n = bytes[4] as usize;
        let ver = bytes.get(5..5 + n).ok_or("bytecode file cut short")?;
        let want = env!("CARGO_PKG_VERSION");
        if ver != want.as_bytes() {
            return Err(format!("bytecode built by mpp {}, this is mpp {want}; rebuild it", String::from_utf8_lossy(ver)));
        }
        postcard::from_bytes(&bytes[5 + n..]).map_err(|e| format!("broken bytecode file: {e}"))
    }
}
