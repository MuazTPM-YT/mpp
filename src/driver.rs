use crate::compile::chunk::Program;
use crate::compile::{self, FileDiags, emit};
use crate::diag::Sources;
use crate::syntax;
use crate::vm::{Flow, Value, Vm, format_trace};
use std::io::Write;
use std::rc::Rc;

pub fn report_diags(sources: &Sources, diags: &FileDiags) {
    for (file, d) in diags {
        sources.report(file, d);
    }
    let n = diags.len();
    eprintln!("{n} error{} found", if n == 1 { "" } else { "s" });
}

// uncaught error: message, code snippet, call stack
pub fn report_flow(sources: &Sources, f: &Flow) -> i32 {
    let e = match f {
        Flow::Exit(code) => return *code,
        Flow::Throw(e) => e,
    };
    let (msg, trace) = match e {
        Value::Error(obj) => (format!("{}: {}", obj.kind, obj.message), obj.trace.borrow().clone()),
        other => (format!("{other:?}"), Vec::new()),
    };
    let notes = if trace.len() > 1 { vec![format!("call stack (newest first):\n{}", format_trace(&trace).trim_end())] } else { vec![] };
    let shown = trace.first().is_some_and(|t| sources.report_at(&t.file, t.line, t.col, &msg, notes.clone()));
    if !shown {
        eprintln!("error: {msg}");
        if !trace.is_empty() {
            eprint!("{}", format_trace(&trace));
        }
    }
    1
}

// compile + run a file; returns exit code
pub fn run_file(path: &str, argv: Vec<String>, seed: u64) -> i32 {
    let mut sources = Sources::default();
    let program = match compile::load_any(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            report_diags(&sources, &d);
            return 2;
        }
    };
    run_program(program, &sources, argv, seed)
}

pub fn run_program(program: Program, sources: &Sources, argv: Vec<String>, seed: u64) -> i32 {
    let out = Box::new(std::io::BufWriter::new(std::io::stdout()));
    let mut vm = Vm::new(program, out);
    vm.argv = argv;
    vm.rng = crate::stdlib::rand::Rng::new(seed);
    let r = vm.run_main();
    let _ = vm.out.flush();
    match r {
        Ok(_) => 0,
        Err(f) => report_flow(sources, &f),
    }
}

// shared in-memory output, for tests and embedding
#[derive(Clone, Default)]
pub struct SharedOut(pub Rc<std::cell::RefCell<Vec<u8>>>);

impl Write for SharedOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedOut {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.borrow()).to_string()
    }
}

// run a source string; output text and the error (rendered) if any
pub fn run_source(name: &str, src: &str) -> (String, Option<String>) {
    let mut sources = Sources::default();
    sources.add(name, src.to_string());
    let ast = match syntax::parse(src) {
        Ok(a) => a,
        Err(ds) => {
            return (String::new(), Some(ds.iter().map(|d| sources.render(name, d)).collect()));
        }
    };
    let m = match emit::compile_module(&ast, name, src, &[], false) {
        Ok(m) => m,
        Err(ds) => {
            return (String::new(), Some(ds.iter().map(|d| sources.render(name, d)).collect()));
        }
    };
    let mut modules = vec![Rc::new(m.proto)];
    let queue = m.imports.into_iter().map(|(k, s)| (k, Some((Rc::from(name), s)))).collect();
    if let Err(ds) = compile::load_into(queue, &mut sources, &mut modules) {
        return (String::new(), Some(ds.iter().map(|(f, d)| sources.render(f, d)).collect()));
    }
    let out = SharedOut::default();
    let mut vm = Vm::new(Program { entry: name.into(), modules }, Box::new(out.clone()));
    vm.rng = crate::stdlib::rand::Rng::new(0);
    let r = vm.run_main();
    let e = r.err().map(|f| match f {
        Flow::Exit(c) => format!("exit {c}"),
        Flow::Throw(Value::Error(o)) => format!("{}: {}", o.kind, o.message),
        Flow::Throw(v) => format!("{v:?}"),
    });
    (out.text(), e)
}

const EXE_MAGIC: &[u8; 8] = b"MPPEXE01";

// copy of this binary with the program glued on the end
pub fn build_exe(program: &Program, out: &str) -> std::io::Result<()> {
    let me = std::env::current_exe()?;
    let mut bytes = std::fs::read(me)?;
    if let Some(n) = embedded_len(&bytes) {
        bytes.truncate(bytes.len() - 16 - n);
    }
    let body = program.to_bytes();
    bytes.extend_from_slice(&body);
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(EXE_MAGIC);
    std::fs::write(out, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn embedded_len(bytes: &[u8]) -> Option<usize> {
    let n = bytes.len();
    if n < 16 || &bytes[n - 8..] != EXE_MAGIC {
        return None;
    }
    let len = u64::from_le_bytes(bytes[n - 16..n - 8].try_into().ok()?) as usize;
    (len <= n - 16).then_some(len)
}

// program glued to this binary, if any
pub fn embedded_program() -> Option<Program> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(std::env::current_exe().ok()?).ok()?;
    let size = f.seek(SeekFrom::End(0)).ok()?;
    if size < 16 {
        return None;
    }
    let mut tail = [0u8; 16];
    f.seek(SeekFrom::End(-16)).ok()?;
    f.read_exact(&mut tail).ok()?;
    if &tail[8..] != EXE_MAGIC {
        return None;
    }
    let len = u64::from_le_bytes(tail[..8].try_into().ok()?);
    if len > size - 16 {
        return None;
    }
    f.seek(SeekFrom::End(-16 - len as i64)).ok()?;
    let mut body = vec![0u8; len as usize];
    f.read_exact(&mut body).ok()?;
    Program::from_bytes(&body).ok()
}
