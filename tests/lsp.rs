// talk to `mpp lsp` over stdio like an editor does
use serde_json::{Value as J, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn send(w: &mut impl Write, v: J) {
    let s = v.to_string();
    write!(w, "Content-Length: {}\r\n\r\n{s}", s.len()).unwrap();
    w.flush().unwrap();
}

fn recv(r: &mut impl BufRead) -> J {
    let mut len = 0;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        if h == "\r\n" {
            break;
        }
        if let Some(v) = h.strip_prefix("Content-Length:") {
            len = v.trim().parse().unwrap();
        }
    }
    let mut buf = vec![0; len];
    r.read_exact(&mut buf).unwrap();
    serde_json::from_slice(&buf).unwrap()
}

// next message that answers `id` (skipping notifications)
fn reply(r: &mut impl BufRead, id: i64) -> J {
    loop {
        let m = recv(r);
        if m["id"] == id {
            return m;
        }
    }
}

#[test]
fn lsp_session() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mpp")).arg("lsp").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut w = child.stdin.take().unwrap();
    let mut r = BufReader::new(child.stdout.take().unwrap());
    send(&mut w, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}));
    let init = reply(&mut r, 1);
    assert_eq!(init["result"]["capabilities"]["hoverProvider"], true);
    send(&mut w, json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
    let uri = "file:///tmp/demo.mpp";
    let text = "import stats\nx=1\nprint(stats.mean([x]), y)\n";
    send(
        &mut w,
        json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": uri, "languageId": "mpp", "version": 1, "text": text}}}),
    );
    let diag = recv(&mut r);
    assert_eq!(diag["method"], "textDocument/publishDiagnostics");
    assert_eq!(diag["params"]["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(diag["params"]["diagnostics"][0]["range"]["start"], json!({"line": 2, "character": 23}));
    send(
        &mut w,
        json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/hover", "params": {"textDocument": {"uri": uri}, "position": {"line": 2, "character": 14}}}),
    );
    let h = reply(&mut r, 2);
    assert!(h["result"]["contents"]["value"].as_str().unwrap().contains("stats.mean"), "{h}");
    send(
        &mut w,
        json!({"jsonrpc": "2.0", "id": 3, "method": "textDocument/formatting", "params": {"textDocument": {"uri": uri}, "options": {"tabSize": 4, "insertSpaces": true}}}),
    );
    let f = reply(&mut r, 3);
    assert_eq!(f["result"][0]["newText"], "import stats\nx = 1\nprint(stats.mean([x]), y)\n");
    send(
        &mut w,
        json!({"jsonrpc": "2.0", "id": 4, "method": "textDocument/completion", "params": {"textDocument": {"uri": uri}, "position": {"line": 2, "character": 12}}}),
    );
    let c = reply(&mut r, 4);
    assert!(c["result"].as_array().unwrap().iter().any(|i| i["label"] == "ttest"));
    send(&mut w, json!({"jsonrpc": "2.0", "id": 5, "method": "shutdown"}));
    reply(&mut r, 5);
    send(&mut w, json!({"jsonrpc": "2.0", "method": "exit"}));
    assert!(child.wait().unwrap().success());
}
