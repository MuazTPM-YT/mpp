// test runner end to end on a fixture file
use mpp::runner::{Options, Status, run_file};

#[test]
fn runner_outcomes() {
    let dir = std::env::temp_dir().join(format!("mpp_runner_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cases_test.mpp");
    std::fs::copy("tests/fixtures/runner_cases.mpp", &file).unwrap();
    let file = file.to_string_lossy().to_string();
    let opts = Options { seed: 1, ..Default::default() };
    let r = run_file(&file, &opts);
    assert_eq!(r.module_output, "module code runs once\n");
    let get = |n: &str| r.results.iter().find(|t| t.name == n).unwrap_or_else(|| panic!("no {n}"));
    assert_eq!(get("passes").status, Status::Passed);
    let f = get("fails");
    assert_eq!(f.status, Status::Failed);
    assert!(f.message.as_ref().unwrap().contains("right: [1, 3]"));
    assert_eq!(f.output, "captured\n");
    assert_eq!(get("errors").status, Status::Error);
    assert_eq!(get("skips").status, Status::Skipped);
    let rep = get("reports");
    assert_eq!(rep.reports.len(), 2);
    assert_eq!(rep.reports[0].label, "rate");
    assert_eq!(rep.reports[1].label, "{\"a\": 1}");
    let p = get("finds smallest");
    assert_eq!(p.status, Status::Failed);
    assert_eq!(p.counterexample.as_deref(), Some("x = -1"));
    let l = get("list shrinks");
    assert_eq!(l.status, Status::Failed);
    // several inputs are "smallest" (e.g. [50] or [25, 25]); all sit right at the limit
    let cx = l.counterexample.as_deref().unwrap();
    let nums: Vec<i64> = cx.trim_start_matches("xs = [").trim_end_matches(']').split(", ").map(|n| n.parse().unwrap()).collect();
    assert!((50..=55).contains(&nums.iter().sum::<i64>()) && nums.len() <= 3, "{cx}");
    assert!(get("times out").message.as_ref().unwrap().contains("TimeoutError"));
    // first run writes the snapshot, second compares
    assert_eq!(get("snap").status, Status::Passed);
    assert!(dir.join("__snapshots__/cases_test.snap.json").exists());
    let again = run_file(&file, &opts);
    assert_eq!(again.results.iter().find(|t| t.name == "snap").unwrap().status, Status::Passed);
    assert!(r.results.iter().all(|t| t.kind != "bench"));
    let b = run_file(&file, &Options { bench: true, ..opts });
    assert_eq!(b.results.len(), 1);
    assert_eq!(b.results[0].bench.as_ref().unwrap().iters, 20);
    std::fs::remove_dir_all(&dir).ok();
}
