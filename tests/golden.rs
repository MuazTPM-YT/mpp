// run every tests/cases/*.mpp, compare to .out; MPP_BLESS=1 rewrites .out
use std::path::Path;

#[test]
fn golden_cases() {
    let bless = std::env::var("MPP_BLESS").is_ok();
    let mut files: Vec<_> = std::fs::read_dir("tests/cases")
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mpp"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let mut failed = Vec::new();
    for path in &files {
        let name = path.to_string_lossy().replace('\\', "/");
        let src = std::fs::read_to_string(path).unwrap();
        let (out, err) = mpp::driver::run_source(&name, &src);
        let mut actual = out;
        if let Some(e) = err {
            actual.push_str(&format!("--- error ---\n{e}\n"));
        }
        let exp_path = path.with_extension("out");
        if bless {
            std::fs::write(&exp_path, &actual).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&exp_path).unwrap_or_default();
        if expected != actual {
            failed.push(format!("== {} ==\n--- expected\n{expected}\n--- actual\n{actual}", Path::new(&name).display()));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
