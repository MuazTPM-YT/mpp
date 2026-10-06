// compare stats against scipy reference values (tests/fixtures/stats_ref.json)
use mpp::stdlib::stats::dist::{self, Alt};
use mpp::stdlib::stats::{desc, regress, tests as t};
use serde_json::Value as J;

fn load() -> J {
    serde_json::from_str(&std::fs::read_to_string("tests/fixtures/stats_ref.json").unwrap()).unwrap()
}

fn v(j: &J) -> Vec<f64> {
    j.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()
}

fn close(name: &str, got: f64, want: f64, tol: f64) {
    let ok = (got - want).abs() <= tol * want.abs().max(1e-300) || (got - want).abs() < 1e-14;
    assert!(ok, "{name}: got {got}, want {want} (rel tol {tol})");
}

fn check(name: &str, r: &t::Rec, stat: &str, want: &J, tol: f64) {
    let w = v(want);
    close(&format!("{name} statistic"), r.get(stat), w[0], tol);
    close(&format!("{name} p"), r.get("p_value"), w[1], tol);
}

#[test]
fn matches_scipy() {
    let j = load();
    let d = |k: &str| v(&j["data"][k]);
    let r = &j["ref"];
    let (a, b) = (d("a"), d("b"));
    let tol = 1e-7;
    check("ttest_1samp", &t::ttest_1samp(&a, 9.5, Alt::Two, 0.95).unwrap(), "statistic", &r["ttest_1samp"], tol);
    check("ttest_1samp greater", &t::ttest_1samp(&a, 9.5, Alt::Greater, 0.95).unwrap(), "statistic", &r["ttest_1samp_greater"], tol);
    let w = t::ttest_ind(&a, &b, false, Alt::Two, 0.95).unwrap();
    check("welch", &w, "statistic", &r["welch"], tol);
    let ci = v(&r["welch_ci"]);
    let Some((_, mpp::vm::Value::List(l))) = w.0.iter().find(|(k, _)| *k == "ci") else { panic!() };
    close("welch ci lo", l.borrow()[0].num("").unwrap(), ci[0], tol);
    close("welch ci hi", l.borrow()[1].num("").unwrap(), ci[1], tol);
    check("student", &t::ttest_ind(&a, &b, true, Alt::Two, 0.95).unwrap(), "statistic", &r["student"], tol);
    check("welch less", &t::ttest_ind(&a, &b, false, Alt::Less, 0.95).unwrap(), "statistic", &r["welch_less"], tol);
    check("paired", &t::ttest_rel(&d("px"), &d("py"), Alt::Two, 0.95).unwrap(), "statistic", &r["paired"], tol);
    let cont: Vec<Vec<f64>> = j["cont"].as_array().unwrap().iter().map(v).collect();
    check("chi2", &t::chi2_contingency(&cont, true).unwrap(), "statistic", &r["chi2"], tol);
    let c22: Vec<Vec<f64>> = j["c22"].as_array().unwrap().iter().map(v).collect();
    check("chi2 yates", &t::chi2_contingency(&c22, true).unwrap(), "statistic", &r["chi2_yates"], tol);
    check("chi2 gof", &t::chi2_gof(&[18.0, 22.0, 30.0, 30.0], Some(&[25.0; 4])).unwrap(), "statistic", &r["chi2_gof"], tol);
    let f = t::fisher_exact([[c22[0][0], c22[0][1]], [c22[1][0], c22[1][1]]], Alt::Two).unwrap();
    check("fisher", &f, "odds_ratio", &r["fisher"], tol);
    close(
        "fisher greater",
        t::fisher_exact([[8.0, 2.0], [1.0, 5.0]], Alt::Greater).unwrap().get("p_value"),
        v(&r["fisher_greater"])[0],
        tol,
    );
    check("mwu asym", &t::mannwhitneyu(&a, &b, Alt::Two, Some("asymptotic")).unwrap(), "statistic", &r["mwu_asym"], tol);
    check("mwu ties", &t::mannwhitneyu(&d("ti"), &d("tj"), Alt::Two, None).unwrap(), "statistic", &r["mwu_ties"], tol);
    check("mwu exact", &t::mannwhitneyu(&d("small_a"), &d("small_b"), Alt::Two, None).unwrap(), "statistic", &r["mwu_exact"], tol);
    check(
        "mwu exact less",
        &t::mannwhitneyu(&d("small_a"), &d("small_b"), Alt::Less, None).unwrap(),
        "statistic",
        &r["mwu_exact_less"],
        tol,
    );
    check("wilcoxon exact", &t::wilcoxon(&d("px"), Some(&d("py")), Alt::Two, None).unwrap(), "statistic", &r["wilcoxon_exact"], tol);
    let (ba, bb) = (d("big_a"), d("big_b"));
    check(
        "wilcoxon asym",
        &t::wilcoxon(&ba[..100], Some(&bb[..100]), Alt::Two, Some("asymptotic")).unwrap(),
        "statistic",
        &r["wilcoxon_asym"],
        tol,
    );
    check("ks exact", &t::ks_2samp(&a, &b, None).unwrap(), "statistic", &r["ks_exact"], 1e-6);
    check("ks small", &t::ks_2samp(&d("small_a"), &d("small_b"), None).unwrap(), "statistic", &r["ks_small"], 1e-6);
    check("ks asym", &t::ks_2samp(&ba, &bb, Some("asymptotic")).unwrap(), "statistic", &r["ks_asym"], 1e-6);
    let g3 = d("g3");
    check("anova", &t::anova(&[a.clone(), b.clone(), g3.clone()]).unwrap(), "statistic", &r["anova"], tol);
    check("kruskal", &t::kruskal(&[d("ti"), d("tj"), g3.clone()]).unwrap(), "statistic", &r["kruskal"], tol);
    check("levene", &t::levene(&[a.clone(), b.clone(), g3.clone()], "median").unwrap(), "statistic", &r["levene"], tol);
    check("levene mean", &t::levene(&[a.clone(), b.clone(), g3.clone()], "mean").unwrap(), "statistic", &r["levene_mean"], tol);
    for (i, s) in j["shapiro_data"].as_array().unwrap().iter().enumerate() {
        check(&format!("shapiro #{i}"), &t::shapiro(&v(s)).unwrap(), "statistic", &r["shapiro"][i], 1e-5);
    }
    let p = t::pearson(&d("px"), &d("py"), Alt::Two, 0.95).unwrap();
    check("pearson", &p, "r", &r["pearson"], tol);
    check("spearman", &t::spearman(&d("px"), &d("py"), Alt::Two).unwrap(), "rho", &r["spearman"], tol);
    check("spearman ties", &t::spearman(&d("ti")[..35], &d("tj"), Alt::Two).unwrap(), "rho", &r["spearman_ties"], tol);
    check("kendall exact", &t::kendall(&d("px"), &d("py")).unwrap(), "tau", &r["kendall_exact"], tol);
    check("kendall ties", &t::kendall(&d("ti")[..35], &d("tj")).unwrap(), "tau", &r["kendall_ties"], tol);
    let lr = regress::linregress(&d("px"), &d("py")).unwrap();
    let w = v(&r["linregress"]);
    for (k, i) in [("slope", 0), ("intercept", 1), ("r", 2), ("p_value", 3), ("stderr", 4), ("intercept_stderr", 5)] {
        close(&format!("linregress {k}"), lr.get(k), w[i], tol);
    }
    close("skew", desc::skew(&a), r["skew"].as_f64().unwrap(), tol);
    close("kurtosis", desc::kurtosis(&a), r["kurtosis"].as_f64().unwrap(), tol);
    let pv = v(&j["pv"]);
    for m in ["bonferroni", "holm", "bh", "by", "hochberg"] {
        let got = t::adjust(&pv, m).unwrap();
        for (g, w) in got.iter().zip(v(&r[format!("adjust_{m}")])) {
            close(&format!("adjust {m}"), *g, w, tol);
        }
    }
    let o = &r["ols"];
    let fit = regress::ols(&d("py"), &[d("px"), v(&o["x2"])], &["px".into(), "x2".into()], true, 0.95).unwrap();
    close("ols r2", fit.get("r2"), o["r2"].as_f64().unwrap(), tol);
    close("ols adj r2", fit.get("adj_r2"), o["adj_r2"].as_f64().unwrap(), tol);
    close("ols f", fit.get("f"), o["f"].as_f64().unwrap(), tol);
    close("ols f p", fit.get("f_p_value"), o["f_p"].as_f64().unwrap(), 1e-6);
    let Some((_, mpp::vm::Value::Map(coef))) = fit.0.iter().find(|(k, _)| *k == "coef") else { panic!() };
    for (i, (_, c)) in coef.borrow().iter().enumerate() {
        let mpp::vm::Value::Map(c) = c else { panic!() };
        let g = |k: &str| c.borrow().get(&mpp::vm::Key::Str(k.into())).unwrap().num("").unwrap();
        close("ols coef", g("estimate"), v(&o["params"])[i], tol);
        close("ols se", g("stderr"), v(&o["bse"])[i], tol);
        close("ols p", g("p_value"), v(&o["pvalues"])[i], 1e-6);
    }
    let ds = &r["dist"];
    close("t_cdf", dist::t_cdf(-2.1, 7.0), ds["t_cdf"].as_f64().unwrap(), 1e-9);
    close("t_ppf", dist::t_ppf(0.975, 12.0), ds["t_ppf"].as_f64().unwrap(), 1e-9);
    close("chi2_sf", dist::chi2_sf(9.3, 4.0), ds["chi2_sf"].as_f64().unwrap(), 1e-9);
    close("f_sf", dist::f_sf(3.2, 2.0, 30.0), ds["f_sf"].as_f64().unwrap(), 1e-9);
    close("beta_ppf", dist::beta_ppf(0.3, 2.5, 7.0), ds["beta_ppf"].as_f64().unwrap(), 1e-8);
    close("binom_cdf", dist::binom_cdf(7.0, 20.0, 0.3), ds["binom_cdf"].as_f64().unwrap(), 1e-9);
    close("poisson_cdf", dist::poisson_cdf(4.0, 2.5), ds["poisson_cdf"].as_f64().unwrap(), 1e-9);
    close("norm_ppf", dist::norm_ppf(0.001), ds["norm_ppf"].as_f64().unwrap(), 1e-9);
    for (x, w) in [0.2, 0.5, 1.0, 1.36, 2.0].iter().zip(v(&ds["kolmogorov_sf"])) {
        close("kolmogorov", dist::kolmogorov_sf(*x), w, 1e-8);
    }
}
