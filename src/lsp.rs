// mpp lsp: diagnostics, hover, completion, go-to-definition, outline, formatting
use crate::compile::{emit, join_key};
use crate::docs;
use crate::syntax::ast::{Import, Stmt, StmtKind};
use crate::syntax::{self, Diag};
use lsp_server::{Connection, Message, Notification, Request, Response};
use serde_json::{Value as J, json};
use std::collections::HashMap;

pub fn run() -> i32 {
    let (conn, io) = Connection::stdio();
    let caps = json!({
        "positionEncoding": "utf-16",
        "textDocumentSync": 1,
        "hoverProvider": true,
        "completionProvider": {"triggerCharacters": ["."]},
        "definitionProvider": true,
        "documentSymbolProvider": true,
        "documentFormattingProvider": true,
    });
    let init = json!({"capabilities": caps, "serverInfo": {"name": "mpp", "version": env!("CARGO_PKG_VERSION")}});
    let started = conn.initialize_start().and_then(|(id, _)| conn.initialize_finish(id, init));
    if let Err(e) = started {
        eprintln!("mpp lsp: initialize failed: {e}");
        return 1;
    }
    let mut docs: HashMap<String, String> = HashMap::new();
    for msg in &conn.receiver {
        match msg {
            Message::Request(req) => {
                if conn.handle_shutdown(&req).unwrap_or(true) {
                    break;
                }
                let id = req.id.clone();
                let result = handle(&req, &docs);
                let _ = conn.sender.send(Message::Response(Response::new_ok(id, result)));
            }
            Message::Notification(n) => {
                if let Some(uri) = on_notify(&n, &mut docs) {
                    let diags = diagnostics(&uri, docs.get(&uri).map_or("", |s| s.as_str()));
                    let note = Notification::new("textDocument/publishDiagnostics".into(), json!({"uri": uri, "diagnostics": diags}));
                    let _ = conn.sender.send(Message::Notification(note));
                }
            }
            Message::Response(_) => {}
        }
    }
    drop(conn);
    let _ = io.join();
    0
}

// returns the uri whose text changed
fn on_notify(n: &Notification, docs: &mut HashMap<String, String>) -> Option<String> {
    let p = &n.params;
    let uri = p["textDocument"]["uri"].as_str()?.to_string();
    match n.method.as_str() {
        "textDocument/didOpen" => {
            docs.insert(uri.clone(), p["textDocument"]["text"].as_str().unwrap_or("").to_string());
            Some(uri)
        }
        "textDocument/didChange" => {
            let text = p["contentChanges"].as_array()?.last()?["text"].as_str()?.to_string();
            docs.insert(uri.clone(), text);
            Some(uri)
        }
        "textDocument/didClose" => {
            docs.remove(&uri);
            None
        }
        _ => None,
    }
}

fn handle(req: &Request, docs: &HashMap<String, String>) -> J {
    let p = &req.params;
    let uri = p["textDocument"]["uri"].as_str().unwrap_or("");
    let Some(src) = docs.get(uri) else { return J::Null };
    let off = || {
        pos_to_offset(src, p["position"]["line"].as_u64().unwrap_or(0) as usize, p["position"]["character"].as_u64().unwrap_or(0) as usize)
    };
    match req.method.as_str() {
        "textDocument/hover" => hover(src, off()).map_or(J::Null, |md| json!({"contents": {"kind": "markdown", "value": md}})),
        "textDocument/completion" => J::Array(completion(src, off())),
        "textDocument/definition" => definition(uri, src, off()).unwrap_or(J::Null),
        "textDocument/documentSymbol" => J::Array(symbols(src)),
        "textDocument/formatting" => match crate::fmt::format(src) {
            Ok(out) if out != *src => {
                let end = offset_to_pos(src, src.len());
                json!([{"range": {"start": {"line": 0, "character": 0}, "end": end}, "newText": out}])
            }
            _ => json!([]),
        },
        _ => J::Null,
    }
}

// ---- positions: LSP uses UTF-16 columns ----

pub fn offset_to_pos(src: &str, off: usize) -> J {
    let off = off.min(src.len());
    let before = &src[..off];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let col: usize = src[start..off].chars().map(char::len_utf16).sum();
    json!({"line": line, "character": col})
}

pub fn pos_to_offset(src: &str, line: usize, col: usize) -> usize {
    let start = if line == 0 { 0 } else { src.match_indices('\n').nth(line - 1).map_or(src.len(), |(i, _)| i + 1) };
    let mut u = 0;
    for (i, ch) in src[start..].char_indices() {
        if u >= col || ch == '\n' {
            return start + i;
        }
        u += ch.len_utf16();
    }
    src.len()
}

fn range(src: &str, a: usize, b: usize) -> J {
    json!({"start": offset_to_pos(src, a), "end": offset_to_pos(src, b.max(a))})
}

// ---- diagnostics ----

pub fn diagnostics(uri: &str, src: &str) -> Vec<J> {
    let key = uri_to_path(uri).unwrap_or_else(|| "file.mpp".into());
    let diags: Vec<Diag> = match syntax::parse(src) {
        Err(ds) => ds,
        Ok(ast) => emit::compile_module(&ast, &key, src, &[], false).err().unwrap_or_default(),
    };
    diags
        .iter()
        .map(|d| {
            let msg = match &d.note {
                Some(n) => format!("{}\n{n}", d.msg),
                None => d.msg.clone(),
            };
            json!({"range": range(src, d.span.start as usize, d.span.end as usize), "severity": 1, "source": "mpp", "message": msg})
        })
        .collect()
}

// ---- words under the cursor ----

fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

// (word, start, receiver before a dot)
fn word_at(src: &str, off: usize) -> Option<(String, usize, Option<String>)> {
    let off = off.min(src.len());
    let start = src[..off].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(off, |(i, _)| i);
    let end = src[off..].char_indices().find(|(_, c)| !is_word(*c)).map_or(src.len(), |(i, _)| off + i);
    let word = src[start..end].to_string();
    let recv = src[..start]
        .strip_suffix('.')
        .map(|b| b.chars().rev().take_while(|c| is_word(*c)).collect::<Vec<_>>().into_iter().rev().collect::<String>());
    Some((word, start, recv.filter(|r| !r.is_empty())))
}

fn entry_md(prefix: &str, e: &docs::Entry) -> String {
    format!("```mpp\n{prefix}{}\n```\n{}", e.signature, e.doc)
}

fn hover(src: &str, off: usize) -> Option<String> {
    let (word, _, recv) = word_at(src, off)?;
    if word.is_empty() {
        return None;
    }
    if let Some(r) = &recv {
        let module = module_alias(src, r);
        if let Some(m) = module {
            return docs::find(&m, &word).map(|e| entry_md(&format!("{m}."), e));
        }
        // method on some value: show every type that has it
        let hits: Vec<String> = docs::all()
            .iter()
            .filter(|e| e.section.starts_with("methods ") && e.name == word)
            .map(|e| entry_md(&format!("{}.", &e.section[8..]), e))
            .collect();
        return (!hits.is_empty()).then(|| hits.join("\n\n---\n\n"));
    }
    if let Some(d) = user_def(src, &word) {
        return Some(d);
    }
    if crate::stdlib::is_module(&word) {
        return Some(format!("module `{word}`: {} functions — `mpp doc {word}.NAME`", docs::section(&word).count()));
    }
    docs::find("prelude", &word).map(|e| entry_md("", e))
}

// `import stats as s` makes s mean stats
fn module_alias(src: &str, name: &str) -> Option<String> {
    if crate::stdlib::is_module(name) {
        return Some(name.to_string());
    }
    let ast = syntax::parse(src).ok()?;
    for s in &ast {
        if let StmtKind::Import(items) = &s.kind {
            for (what, alias, _) in items {
                if let (Import::Builtin(m), Some(a)) = (what, alias)
                    && &**a == name
                {
                    return Some(m.to_string());
                }
            }
        }
    }
    None
}

// signature + comment line above a user fn/class
fn user_def(src: &str, name: &str) -> Option<String> {
    let ast = syntax::parse(src).ok()?;
    let (start, _) = find_def(&ast, src, name)?;
    let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = src[start..].find('\n').map_or(src.len(), |i| start + i);
    let head = src[line_start..line_end].trim().trim_end_matches('{').trim();
    let above = src[..line_start.saturating_sub(1)].lines().last().map(str::trim).filter(|l| l.starts_with('#'));
    Some(match above {
        Some(c) => format!("```mpp\n{head}\n```\n{}", c.trim_start_matches('#').trim()),
        None => format!("```mpp\n{head}\n```"),
    })
}

// byte range of the name in its definition
fn find_def(stmts: &[Stmt], src: &str, name: &str) -> Option<(usize, usize)> {
    let at = |span_start: u32, word: &str| -> Option<(usize, usize)> {
        let s = span_start as usize;
        let i = src[s..].find(word)? + s;
        Some((i, i + word.len()))
    };
    for s in stmts {
        match &s.kind {
            StmtKind::Fn(f) if &*f.name == name => return at(s.span.start, name),
            StmtKind::Class { name: n, methods, .. } => {
                if &**n == name {
                    return at(s.span.start, name);
                }
                for m in methods {
                    if &*m.name == name {
                        return at(m.span.start, name);
                    }
                }
            }
            StmtKind::Const(n, _) if &**n == name => return at(s.span.start, name),
            StmtKind::Assign(targets, _) => {
                for t in targets {
                    if let syntax::ast::ExprKind::Name(n) = &t.kind
                        && &**n == name
                    {
                        return Some((t.span.start as usize, t.span.end as usize));
                    }
                }
            }
            StmtKind::If(arms, other) => {
                for (_, b) in arms {
                    if let Some(r) = find_def(b, src, name) {
                        return Some(r);
                    }
                }
                if let Some(r) = other.as_ref().and_then(|b| find_def(b, src, name)) {
                    return Some(r);
                }
            }
            StmtKind::For(vars, _, body) => {
                if let Some((_, sp)) = vars.iter().find(|(v, _)| &**v == name) {
                    return Some((sp.start as usize, sp.end as usize));
                }
                if let Some(r) = find_def(body, src, name) {
                    return Some(r);
                }
            }
            StmtKind::While(_, b) | StmtKind::TestBlock { body: b, .. } => {
                if let Some(r) = find_def(b, src, name) {
                    return Some(r);
                }
            }
            StmtKind::Try(b, _, c) => {
                if let Some(r) = find_def(b, src, name).or_else(|| find_def(c, src, name)) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

fn definition(uri: &str, src: &str, off: usize) -> Option<J> {
    let ast = syntax::parse(src).ok()?;
    // `import "lib/x.mpp"`: jump to the file
    for s in &ast {
        if let StmtKind::Import(items) = &s.kind {
            for (what, _, span) in items {
                if let Import::File(p) = what
                    && (span.start as usize..=span.end as usize).contains(&off)
                {
                    let here = uri_to_path(uri)?;
                    let target = join_key(&here, p);
                    let abs = std::fs::canonicalize(&target).ok()?;
                    return Some(
                        json!({"uri": path_to_uri(&abs.to_string_lossy()), "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}}),
                    );
                }
            }
        }
    }
    let (word, _, recv) = word_at(src, off)?;
    if recv.is_some() && !src[..off].ends_with(&word) {
        return None;
    }
    let (a, b) = find_def(&ast, src, &word)?;
    Some(json!({"uri": uri, "range": range(src, a, b)}))
}

const KEYWORDS: &[&str] = &[
    "fn",
    "return",
    "if",
    "elif",
    "else",
    "while",
    "for",
    "in",
    "break",
    "continue",
    "class",
    "import",
    "as",
    "const",
    "try",
    "catch",
    "throw",
    "nil",
    "true",
    "false",
    "and",
    "or",
    "not",
    "global",
    "nonlocal",
    "super",
    "test",
    "experiment",
    "bench",
    "property",
    "expect",
    "report",
    "within",
];

fn item(label: &str, kind: u32, detail: &str, doc: &str) -> J {
    json!({"label": label, "kind": kind, "detail": detail, "documentation": {"kind": "markdown", "value": doc}})
}

fn completion(src: &str, off: usize) -> Vec<J> {
    let Some((_, _, recv)) = word_at(src, off) else { return vec![] };
    if let Some(r) = recv {
        if let Some(m) = module_alias(src, &r) {
            return docs::section(&m).map(|e| item(e.name, 3, e.signature, e.doc)).collect();
        }
        let mut seen = std::collections::HashSet::new();
        return docs::all()
            .iter()
            .filter(|e| e.section.starts_with("methods ") && seen.insert(e.name))
            .map(|e| item(e.name, 2, &format!("{}.{}", &e.section[8..], e.signature), e.doc))
            .collect();
    }
    let mut out: Vec<J> = KEYWORDS.iter().map(|k| json!({"label": k, "kind": 14})).collect();
    out.extend(docs::section("prelude").map(|e| item(e.name, 3, e.signature, e.doc)));
    out.extend(crate::stdlib::MODULES.iter().map(|m| json!({"label": m, "kind": 9, "detail": format!("import {m}")})));
    if let Ok(ast) = syntax::parse(src) {
        let mut names = Vec::new();
        collect_names(&ast, &mut names);
        names.sort();
        names.dedup();
        out.extend(names.into_iter().map(|n| json!({"label": n, "kind": 6})));
    }
    out
}

fn collect_names(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Fn(f) => out.push(f.name.to_string()),
            StmtKind::Class { name, .. } | StmtKind::Const(name, _) => out.push(name.to_string()),
            StmtKind::Assign(ts, _) => {
                for t in ts {
                    if let syntax::ast::ExprKind::Name(n) = &t.kind {
                        out.push(n.to_string());
                    }
                }
            }
            StmtKind::If(arms, other) => {
                for (_, b) in arms {
                    collect_names(b, out);
                }
                if let Some(b) = other {
                    collect_names(b, out);
                }
            }
            StmtKind::For(vars, _, b) => {
                out.extend(vars.iter().map(|v| v.0.to_string()));
                collect_names(b, out);
            }
            StmtKind::While(_, b) | StmtKind::TestBlock { body: b, .. } => collect_names(b, out),
            _ => {}
        }
    }
}

// outline: functions, classes (with methods), test blocks, consts
fn symbols(src: &str) -> Vec<J> {
    let Ok(ast) = syntax::parse(src) else { return vec![] };
    let sym = |name: &str, kind: u32, a: usize, b: usize, children: Vec<J>| json!({"name": name, "kind": kind, "range": range(src, a, b), "selectionRange": range(src, a, b), "children": children});
    ast.iter()
        .filter_map(|s| {
            let (a, b) = (s.span.start as usize, s.span.end as usize);
            match &s.kind {
                StmtKind::Fn(f) => Some(sym(&f.name, 12, a, b, vec![])),
                StmtKind::Class { name, methods, .. } => {
                    let kids = methods.iter().map(|m| sym(&m.name, 6, m.span.start as usize, m.span.end as usize, vec![])).collect();
                    Some(sym(name, 5, a, b, kids))
                }
                StmtKind::Const(n, _) => Some(sym(n, 14, a, b, vec![])),
                StmtKind::TestBlock { kind, name, .. } => Some(sym(&format!("{} \"{name}\"", kind.word()), 24, a, b, vec![])),
                _ => None,
            }
        })
        .collect()
}

// ---- file uris ----

pub fn uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%' && i + 3 <= bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        if let Some(b) = hex {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

pub fn path_to_uri(path: &str) -> String {
    let mut s = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_round_trip() {
        let src = "a = 1\nb = \"é😀\" + x\n";
        let off = src.find('x').unwrap();
        let p = offset_to_pos(src, off);
        assert_eq!(p, json!({"line": 1, "character": 12}));
        assert_eq!(pos_to_offset(src, 1, 12), off);
        assert_eq!(uri_to_path("file:///tmp/a%20b.mpp").unwrap(), "/tmp/a b.mpp");
        assert_eq!(path_to_uri("/tmp/a b.mpp"), "file:///tmp/a%20b.mpp");
    }

    #[test]
    fn features() {
        let src = "import stats as s\n# adds one\nfn inc(x) { return x + 1 }\ny = inc(2)\nz = s.ttest([1], [2])\nprint(zz)\n";
        let d = diagnostics("file:///t.mpp", src);
        assert_eq!(d.len(), 1);
        assert!(d[0]["message"].as_str().unwrap().contains("zz"));
        let h = hover(src, src.find("inc(2)").unwrap() + 1).unwrap();
        assert!(h.contains("fn inc(x)") && h.contains("adds one"), "{h}");
        let h = hover(src, src.find("ttest").unwrap() + 2).unwrap();
        assert!(h.contains("s.ttest") || h.contains("stats.ttest"), "{h}");
        assert!(hover(src, src.find("print").unwrap()).unwrap().contains("Write values"));
        let c = completion(src, src.find("ttest").unwrap());
        assert!(c.iter().any(|i| i["label"] == "mannwhitney"));
        let def = definition("file:///t.mpp", src, src.find("inc(2)").unwrap()).unwrap();
        assert_eq!(def["range"]["start"]["line"], 2);
        assert_eq!(symbols(src)[0]["name"], "inc");
    }
}
