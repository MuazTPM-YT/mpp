pub mod chunk;
pub mod emit;

use crate::diag::Sources;
use crate::syntax::{self, Diag, Span};
use chunk::{ModuleProto, Program};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

pub type FileDiags = Vec<(Rc<str>, Diag)>;
// module key to load, plus the file and span that imported it
pub type ImportQueue = Vec<(Rc<str>, Option<(Rc<str>, Span)>)>;

// import path relative to the importing file, cleaned up
pub fn join_key(cur: &str, path: &str) -> String {
    let p = Path::new(path);
    let full = if p.is_absolute() { p.to_path_buf() } else { Path::new(cur).parent().unwrap_or(Path::new("")).join(p) };
    let mut out = PathBuf::new();
    for c in full.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out.to_string_lossy().replace('\\', "/")
}

// compile entry file and every file it imports
pub fn load_program(entry: &str, sources: &mut Sources) -> Result<Program, FileDiags> {
    let key: Rc<str> = join_key("", entry).into();
    let mut modules = Vec::new();
    load_into(vec![(key.clone(), None)], sources, &mut modules)?;
    Ok(Program { entry: key, modules })
}

// load each queued (key, importer) not already in `modules`
pub fn load_into(mut queue: ImportQueue, sources: &mut Sources, modules: &mut Vec<Rc<ModuleProto>>) -> Result<(), FileDiags> {
    let mut diags = FileDiags::new();
    let mut seen: Vec<Rc<str>> = modules.iter().map(|m| m.key.clone()).collect();
    while let Some((key, from)) = queue.pop() {
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        let src = match std::fs::read_to_string(&*key) {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("cannot read `{key}`: {e}");
                match from {
                    Some((file, span)) => diags.push((file, Diag::new(msg, span))),
                    None => diags.push((key.clone(), Diag::new(msg, Span::default()))),
                }
                continue;
            }
        };
        sources.add(&key, src.clone());
        let out = syntax::parse(&src).and_then(|ast| emit::compile_module(&ast, &key, &src, &[], false));
        match out {
            Ok(m) => {
                for (k, span) in m.imports {
                    queue.push((k, Some((key.clone(), span))));
                }
                modules.push(Rc::new(m.proto));
            }
            Err(ds) => diags.extend(ds.into_iter().map(|d| (key.clone(), d))),
        }
    }
    if diags.is_empty() { Ok(()) } else { Err(diags) }
}

// read a .mpp or .mppc file into a program
pub fn load_any(path: &str, sources: &mut Sources) -> Result<Program, FileDiags> {
    if let Some(bytes) = std::fs::read(path).ok().filter(|b| Program::is_bytecode(b)) {
        return Program::from_bytes(&bytes).map_err(|e| vec![(Rc::from(path), Diag::new(e, Span::default()))]);
    }
    load_program(path, sources)
}

#[cfg(test)]
mod tests {
    use super::join_key;

    #[test]
    fn keys() {
        assert_eq!(join_key("", "main.mpp"), "main.mpp");
        assert_eq!(join_key("ex/main.mpp", "lib/h.mpp"), "ex/lib/h.mpp");
        assert_eq!(join_key("ex/lib/h.mpp", "../x.mpp"), "ex/x.mpp");
        assert_eq!(join_key("main.mpp", "../x.mpp"), "../x.mpp");
        assert_eq!(join_key("./a/./b.mpp", "c.mpp"), "a/c.mpp");
    }
}
