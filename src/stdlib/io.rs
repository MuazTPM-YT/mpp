use crate::vm::*;
use std::io::Write;

pub static FNS: &[Native] = &[
    Native { name: "read", f: read },
    Native { name: "write", f: write },
    Native { name: "append", f: append },
    Native { name: "lines", f: lines },
    Native { name: "exists", f: exists },
    Native { name: "ls", f: ls },
    Native { name: "mkdir", f: mkdir },
    Native { name: "remove", f: remove },
];

fn io_err(path: &str, e: std::io::Error) -> Flow {
    err("IOError", format!("{path}: {e}"))
}

fn path(v: Option<Value>) -> Result<String, Flow> {
    Ok(need(v, "path")?.as_str("path")?.to_string())
}

pub fn read_text(p: &str) -> Result<String, Flow> {
    std::fs::read_to_string(p).map_err(|e| io_err(p, e))
}

fn read(_: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    Ok(Value::str(read_text(&path(p)?)?))
}

fn write(vm: &mut Vm, a: Args) -> R {
    let [p, text] = a.bind(["path", "text"])?;
    let p = path(p)?;
    let t = vm.display(&need(text, "text")?, false)?;
    std::fs::write(&p, t).map_err(|e| io_err(&p, e))?;
    Ok(Value::Nil)
}

fn append(vm: &mut Vm, a: Args) -> R {
    let [p, text] = a.bind(["path", "text"])?;
    let p = path(p)?;
    let t = vm.display(&need(text, "text")?, false)?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).map_err(|e| io_err(&p, e))?;
    f.write_all(t.as_bytes()).map_err(|e| io_err(&p, e))?;
    Ok(Value::Nil)
}

fn lines(_: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    Ok(Value::list(read_text(&path(p)?)?.lines().map(Value::str).collect()))
}

fn exists(_: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    Ok(Value::Bool(std::path::Path::new(&path(p)?).exists()))
}

fn ls(_: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    let p = opt(p).map_or(Ok(".".to_string()), |v| Ok::<_, Flow>(v.as_str("path")?.to_string()))?;
    let mut names: Vec<String> = std::fs::read_dir(&p)
        .map_err(|e| io_err(&p, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    Ok(Value::list(names.into_iter().map(Value::str).collect()))
}

fn mkdir(_: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    let p = path(p)?;
    std::fs::create_dir_all(&p).map_err(|e| io_err(&p, e))?;
    Ok(Value::Nil)
}

fn remove(_: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    let p = path(p)?;
    let meta = std::fs::metadata(&p).map_err(|e| io_err(&p, e))?;
    if meta.is_dir() { std::fs::remove_dir(&p) } else { std::fs::remove_file(&p) }.map_err(|e| io_err(&p, e))?;
    Ok(Value::Nil)
}
