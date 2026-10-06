// ml metrics vs scikit-learn / scipy / statsmodels (tests/fixtures/ml_ref.json)
use serde_json::Value as J;

#[test]
fn ml_matches_sklearn() {
    let want: J = serde_json::from_str(&std::fs::read_to_string("tests/fixtures/ml_ref.json").unwrap()).unwrap();
    let src = r#"
import ml, json
d = json.load("tests/fixtures/ml_ref.json").data
cal = ml.calibration(d.y, d.score)
print(json.dump({
    "accuracy": ml.accuracy(d.y, d.pred),
    "balanced_accuracy": ml.balanced_accuracy(d.ym, d.pm),
    "precision": ml.precision(d.y, d.pred), "recall": ml.recall(d.y, d.pred), "f1": ml.f1(d.y, d.pred),
    "f1_macro": ml.f1(d.ym, d.pm), "f1_weighted": ml.f1(d.ym, d.pm, average = "weighted"),
    "precision_micro": ml.precision(d.ym, d.pm, average = "micro"), "fbeta2": ml.fbeta(d.y, d.pred, beta = 2),
    "mcc": ml.mcc(d.ym, d.pm), "kappa": ml.cohen_kappa(d.ym, d.pm),
    "confusion": ml.confusion_matrix(d.ym, d.pm).matrix,
    "roc_auc": ml.roc_auc(d.y, d.score), "roc_auc_ovr": ml.roc_auc(d.ym, d.probs),
    "ap": ml.average_precision(d.y, d.score), "log_loss": ml.log_loss(d.y, d.score),
    "log_loss_multi": ml.log_loss(d.ym, d.probs), "brier": ml.brier(d.y, d.score),
    "top2": ml.top_k_accuracy(d.ym, d.probs, k = 2),
    "mae": ml.mae(d.yr, d.pr), "mse": ml.mse(d.yr, d.pr), "r2": ml.r2(d.yr, d.pr), "mape": ml.mape(d.yr, d.pr),
    "median_ae": ml.regression(d.yr, d.pr).median_ae, "max_error": ml.regression(d.yr, d.pr).max_error,
    "explained_variance": ml.regression(d.yr, d.pr).explained_variance,
    "ndcg": ml.ndcg(d.rels), "ndcg3": ml.ndcg(d.rels, k = 3), "ndcg_sklearn_q0": ml.ndcg(d.rels[0]),
    "cal_frac_pos": cal.bins.frac_pos, "cal_mean_pred": cal.bins.mean_pred,
    "wasserstein": ml.wasserstein(d.ref, d.cur), "psi": ml.psi(d.ref, d.cur).psi,
    "kl": ml.kl(d.p, d.q), "js": ml.js(d.p, d.q),
    "mcnemar_exact_p": ml.mcnemar(d.y, d.pred, d.pred2, exact = true).p_value,
    "mcnemar_chi2_p": ml.mcnemar(d.y, d.pred, d.pred2, exact = false).p_value,
    "auc2": ml.delong(d.y, d.score, d.score2).auc_b,
    "delong_z": ml.delong(d.y, d.score, d.score2).z, "delong_p": ml.delong(d.y, d.score, d.score2).p_value,
}))
"#;
    let (out, err) = mpp::driver::run_source("ml_ref.mpp", src);
    assert!(err.is_none(), "{err:?}");
    let got: J = serde_json::from_str(out.trim()).unwrap();
    let r = want["ref"].as_object().unwrap();
    let mut bad = Vec::new();
    for (k, w) in r {
        let g = &got[k];
        let pairs: Vec<(f64, f64)> = match (g, w) {
            (J::Array(a), J::Array(b)) => {
                let flat = |x: &Vec<J>| -> Vec<f64> {
                    x.iter()
                        .flat_map(|v| {
                            if let J::Array(i) = v { i.iter().map(|z| z.as_f64().unwrap()).collect() } else { vec![v.as_f64().unwrap()] }
                        })
                        .collect()
                };
                let (fa, fb) = (flat(a), flat(b));
                if fa.len() != fb.len() {
                    bad.push(format!("{k}: length {} vs {}", fa.len(), fb.len()));
                    continue;
                }
                fa.into_iter().zip(fb).collect()
            }
            _ => vec![(g.as_f64().unwrap_or(f64::NAN), w.as_f64().unwrap())],
        };
        for (a, b) in pairs {
            if (a - b).abs() > 1e-9 * b.abs().max(1.0) {
                bad.push(format!("{k}: got {a}, want {b}"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
