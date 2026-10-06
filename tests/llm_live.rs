// llm kit against a fake OpenAI-style server and a fake model script
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

fn answer(prompt: &str) -> String {
    if prompt.contains("2+2") {
        "4".into()
    } else if prompt.contains("France") {
        "Paris".into()
    } else if prompt.contains("strict grader") {
        "Score: 8".into()
    } else {
        format!("echo: {prompt}")
    }
}

fn handle(mut s: TcpStream) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut first = String::new();
    r.read_line(&mut first).unwrap();
    let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut len = 0;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        if h == "\r\n" || h.is_empty() {
            break;
        }
        if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).unwrap();
    let req: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    let ok = |s: &mut TcpStream, b: String| {
        let _ =
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len());
    };
    match path.as_str() {
        "/v1/chat/completions" => {
            let msgs = req["messages"].as_array().cloned().unwrap_or_default();
            let prompt = msgs.last().and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
            let text = answer(&prompt);
            if req["stream"] == true {
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                for w in text.split(' ') {
                    let chunk = serde_json::json!({"choices": [{"delta": {"content": format!("{w} ")}}]});
                    let _ = write!(s, "data: {chunk}\n\n");
                    let _ = s.flush();
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                let _ = write!(s, "data: [DONE]\n\n");
                return;
            }
            let lp: Vec<_> = text.split(' ').map(|w| serde_json::json!({"token": w, "logprob": -0.5})).collect();
            let mut reply = serde_json::json!({"choices": [{"message": {"content": text}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": prompt.split_whitespace().count(), "completion_tokens": text.split_whitespace().count()}});
            if req["logprobs"] == true {
                reply["choices"][0]["logprobs"] = serde_json::json!({"content": lp});
            }
            ok(&mut s, reply.to_string());
        }
        "/v1/completions" => {
            let prompt = req["prompt"].as_str().unwrap_or("");
            let words: Vec<&str> = prompt.split_whitespace().collect();
            let mut toks: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            toks.push("x".into());
            let mut lps: Vec<serde_json::Value> = vec![serde_json::Value::Null];
            lps.extend((1..toks.len()).map(|_| serde_json::json!(-1.0)));
            let reply = serde_json::json!({"choices": [{"text": format!("{prompt} x"), "logprobs": {"tokens": toks, "token_logprobs": lps}}],
                "usage": {"prompt_tokens": words.len(), "completion_tokens": 1}});
            ok(&mut s, reply.to_string());
        }
        "/raw" => ok(&mut s, serde_json::json!({"output": {"text": answer(req["prompt"].as_str().unwrap_or(""))}}).to_string()),
        _ => {
            let _ = write!(s, "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\nboom");
        }
    }
}

fn server() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            std::thread::spawn(move || handle(s));
        }
    });
    format!("http://{addr}")
}

fn run(src: &str) -> String {
    let (out, err) = mpp::driver::run_source("llm_live.mpp", src);
    assert!(err.is_none(), "{err:?}\n{out}");
    out
}

#[test]
fn http_model_end_to_end() {
    let url = server();
    let out = run(&format!(
        r#"
import llm
m = llm.http("{url}", model = "tiny", temperature = 0)
r = m.generate("what is 2+2?")
print(r.text, r.tokens_in, r.tokens_out, r.finish_reason, r.latency_ms > 0)
print(m.ask("capital of France?"))
s = m.stream("hello there friend")
print(s.text.strip(), s.ttft_ms != nil, s.tokens_out)
print(m.logprobs("2+2")[0].logprob)
print(len(m.batch(["2+2", "France", "x"], concurrency = 3)))
c = llm.http("{url}", api = "completions")
print(c.perplexity("a b c d").perplexity)
raw = llm.http("{url}/raw", mode = "raw", field = "output.text")
print(raw.ask("France?"))
cases = [{{"prompt": "2+2?", "expected": "4"}}, {{"prompt": "capital of France", "expected": "paris"}}, {{"prompt": "say hi", "expected": "hi"}}]
e = llm.eval_set(m, cases, metric = "exact")
print(e.score, e.passed, e.n, e.results.nrows)
j = llm.judge(m, "Paris", "Is the answer correct?")
print(j.rating, j.score)
lt = llm.load_test(m, ["2+2", "France"], requests = 20, concurrency = 4)
print(lt.ok, lt.errors, lt.latency_ms.n)
bad = llm.http("{url}/nope", mode = "raw")
try {{ bad.ask("x") }} catch e {{ print(e.kind, "500" in e.message) }}
down = llm.http("http://127.0.0.1:1", timeout = 2)
try {{ down.ask("x") }} catch e {{ print(e.kind) }}
lt2 = llm.load_test(down, ["x"], requests = 3)
print(lt2.errors, lt2.error_rate)
d = llm.determinism(m, "2+2", runs = 3)
print(d.deterministic, d.unique_outputs)
"#
    ));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "4 3 1 stop true");
    assert_eq!(lines[1], "Paris");
    assert_eq!(lines[2], "echo: hello there friend true 4");
    assert_eq!(lines[3], "-0.5");
    assert_eq!(lines[4], "3");
    assert_eq!(lines[5], format!("{}", 1f64.exp()));
    assert_eq!(lines[6], "Paris");
    assert_eq!(lines[7], format!("{} 2 3 3", 2.0 / 3.0));
    assert_eq!(lines[8], format!("8.0 {}", 7.0 / 9.0));
    assert_eq!(lines[9], "20 0 20");
    assert_eq!(lines[10], "ModelError true");
    assert_eq!(lines[11], "ModelError");
    assert_eq!(lines[12], "3 1.0");
    assert_eq!(lines[13], "true 1");
}

#[test]
fn process_model_and_checkpoints() {
    let out = run(r#"
import llm
p = llm.process("python3 tests/fixtures/mock_model.py")
print(p.ask("capital of France?"))
print(p.generate("hi", logprobs = true).logprobs[0].logprob)
a = llm.mock((prompt, params) => "Paris")
b = llm.mock((prompt, params) => "London")
cases = [{"prompt": "capital of France", "expected": "Paris"}] * 10
cmp = llm.compare_checkpoints(a, b, cases, metric = "exact")
print(cmp.score_a, cmp.score_b, cmp.verdict, cmp.regressions.nrows)
same = llm.compare_checkpoints(a, a, cases)
print(same.verdict)
c = llm.consistency(p, ["capital of France?", "France capital?"])
print(c.agreement)
"#);
    assert_eq!(out, "Paris\n-0.25\n1.0 0.0 worse 10\nno clear difference\n1.0\n");
}
