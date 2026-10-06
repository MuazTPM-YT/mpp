// A/B, power and sequential numbers vs statsmodels / scipy / published tables
use serde_json::Value as J;

fn run(src: &str) -> J {
    let (out, err) = mpp::driver::run_source("ab_ref.mpp", src);
    assert!(err.is_none(), "{err:?}\n{out}");
    serde_json::from_str(out.trim()).unwrap()
}

fn close(name: &str, got: &J, want: f64, tol: f64) {
    let g = got.as_f64().unwrap_or_else(|| panic!("{name}: not a number: {got}"));
    assert!((g - want).abs() <= tol * want.abs().max(1e-12), "{name}: got {g}, want {want}");
}

#[test]
fn ab_power_sequential() {
    let want: J = serde_json::from_str(&std::fs::read_to_string("tests/fixtures/ab_ref.json").unwrap()).unwrap();
    let got = run(r#"
import ab, power, json
r = ab.proportions(x_a = 100, n_a = 1000, x_b = 130, n_b = 1000)
g = ab.proportions(x_a = 100, n_a = 1000, x_b = 130, n_b = 1000, alternative = "greater")
b = ab.bayes(x_a = 30, n_a = 100, x_b = 45, n_b = 100)
s = ab.sequential_bounds(looks = 5)
print(json.dump({
    "z": r.z, "p": r.p_value, "ci": r.ci_diff, "pg": g.p_value,
    "pm": power.achieved_means(1.0, 0.2, 300, ratio = 1.5),
    "pm1": power.achieved_means(1.0, 0.2, 300, alternative = "greater"),
    "nm": power.means(1.0, 0.25).n_a,
    "np": power.proportions(0.10, 0.02, relative = false).n_a,
    "pb": b.prob_b_better,
    "obf": s.looks.map(l => l.z_bound),
    "spent": s.looks[-1].alpha_spent,
}))
"#);
    let tol = 1e-9;
    close("z", &got["z"], want["prop"]["z"].as_f64().unwrap(), tol);
    close("p", &got["p"], want["prop"]["p"].as_f64().unwrap(), tol);
    close("ci lo", &got["ci"][0], want["prop"]["ci"][0].as_f64().unwrap(), tol);
    close("ci hi", &got["ci"][1], want["prop"]["ci"][1].as_f64().unwrap(), tol);
    close("p greater", &got["pg"], want["prop_greater_p"].as_f64().unwrap(), tol);
    close("power means", &got["pm"], want["power_means"].as_f64().unwrap(), tol);
    close("power means one-sided", &got["pm1"], want["power_means_one"].as_f64().unwrap(), tol);
    assert_eq!(got["nm"].as_f64().unwrap(), want["n_means"].as_f64().unwrap().ceil());
    assert_eq!(got["np"].as_f64().unwrap(), want["n_props_formula"].as_f64().unwrap().ceil());
    close("prob b better", &got["pb"], want["prob_b_better"].as_f64().unwrap(), 1e-9);
    // Lan-DeMets O'Brien-Fleming, K=5, two-sided alpha 0.05 (gsDesign / ldbounds)
    for (i, w) in [4.8769, 3.3569, 2.6803, 2.2898, 2.0310].iter().enumerate() {
        close(&format!("obf look {}", i + 1), &got["obf"][i], *w, 5e-4);
    }
    close("alpha spent", &got["spent"], 0.05, 1e-6);
}
