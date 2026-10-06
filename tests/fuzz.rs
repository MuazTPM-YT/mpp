// broken input must give errors, never a crash
use mpp::stdlib::rand::Rng;

const BITS: &[&str] = &[
    "fn", "f", "(", ")", "{", "}", "[", "]", "x", "=", "==", "+", "-", "*", "**", "/", "//", "%", ",", ".", "..", "..=", ":", ";", "?",
    "=>", "\n", " ", "if", "elif", "else", "for", "in", "while", "return", "class", "super", "try", "catch", "throw", "import", "\"s\"",
    "f\"{x}\"", "f\"{", "1", "2.5", "nil", "not", "and", "or", "~=", "global", "nonlocal", "const", "break", "continue", "'", "\"", "#c",
    "x =>", "a, b", "\\", "\u{e9}",
];

fn check(src: &str) {
    if let Ok(ast) = mpp::syntax::parse(src) {
        let _ = mpp::compile::emit::compile_module(&ast, "fuzz.mpp", src, &[], false);
    }
}

#[test]
fn random_soup_never_panics() {
    let mut r = Rng::new(1);
    for _ in 0..20_000 {
        let n = 1 + r.below(40) as usize;
        let src: String = (0..n).map(|_| BITS[r.below(BITS.len() as u64) as usize]).collect::<Vec<_>>().join(" ");
        check(&src);
    }
}

#[test]
fn mangled_cases_never_panic() {
    let mut r = Rng::new(2);
    for e in std::fs::read_dir("tests/cases").unwrap().flatten() {
        let Ok(src) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        let chars: Vec<char> = src.chars().collect();
        for _ in 0..300 {
            let mut c = chars.clone();
            for _ in 0..1 + r.below(4) {
                if c.is_empty() {
                    break;
                }
                let i = r.below(c.len() as u64) as usize;
                match r.below(3) {
                    0 => {
                        c.remove(i);
                    }
                    1 => c.insert(i, ['{', '}', '(', '"', '\n', '=', 'f'][r.below(7) as usize]),
                    _ => c.truncate(i),
                }
            }
            check(&c.into_iter().collect::<String>());
        }
    }
}
