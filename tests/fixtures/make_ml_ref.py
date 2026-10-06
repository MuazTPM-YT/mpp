# Regenerate ml_ref.json: python make_ml_ref.py > ml_ref.json (needs scikit-learn, scipy, statsmodels)
import json, numpy as np
from sklearn import metrics as M
from sklearn.calibration import calibration_curve
from scipy import stats
from scipy.spatial.distance import jensenshannon
from statsmodels.stats.contingency_tables import mcnemar

rng = np.random.default_rng(3)
n = 300
y = rng.integers(0, 2, n)
score = np.clip(y * 0.35 + rng.normal(0.4, 0.22, n), 0.001, 0.999).round(4)
score2 = np.clip(y * 0.25 + rng.normal(0.42, 0.25, n), 0.001, 0.999).round(4)
pred = (score >= 0.55).astype(int)
pred2 = (score2 >= 0.55).astype(int)
ym = rng.integers(0, 3, n)
pm = np.where(rng.random(n) < 0.7, ym, rng.integers(0, 3, n))
probs = rng.dirichlet([1, 1, 1], n)
probs[np.arange(n), ym] += 0.6
probs = (probs / probs.sum(1, keepdims=True)).round(5)
yr = rng.normal(50, 10, 120).round(3)
pr = (yr + rng.normal(0, 4, 120)).round(3)
rels = [[3, 2, 3, 0, 1, 2], [0, 0, 1, 0, 2], [1, 0, 0, 0]]
ref = rng.normal(0, 1, 500).round(4); cur = rng.normal(0.3, 1.2, 400).round(4)
pdist = [0.1, 0.4, 0.5]; qdist = [0.3, 0.3, 0.4]

def ndcg_lin(r, k):
    r = np.asarray(r, float)
    dcg = lambda x: sum(v / np.log2(i + 2) for i, v in enumerate(x[:k]))
    ideal = dcg(np.sort(r)[::-1])
    return 0.0 if ideal == 0 else dcg(r) / ideal

def psi(e, a, bins=10):
    edges = np.unique(np.quantile(e, np.arange(1, bins) / bins))
    def share(x):
        idx = np.searchsorted(edges, x, side="left")
        c = np.bincount(idx, minlength=len(edges) + 1) / len(x)
        return np.maximum(c, 1e-4)
    s1, s2 = share(e), share(a)
    return float(np.sum((s2 - s1) * np.log(s2 / s1)))

def delong(y, s1, s2):
    pos = y == 1
    def comps(s):
        X, Y = s[pos], s[~pos]
        psi = (X[:, None] > Y[None, :]) + 0.5 * (X[:, None] == Y[None, :])
        return psi.mean(1), psi.mean(0), psi.mean()
    v10a, v01a, a1 = comps(s1)
    v10b, v01b, a2 = comps(s2)
    S10 = np.cov(np.vstack([v10a, v10b])); S01 = np.cov(np.vstack([v01a, v01b]))
    var = (S10[0, 0] + S10[1, 1] - 2 * S10[0, 1]) / pos.sum() + (S01[0, 0] + S01[1, 1] - 2 * S01[0, 1]) / (~pos).sum()
    z = (a2 - a1) / np.sqrt(var)
    return float(z), float(2 * stats.norm.sf(abs(z)))

dz, dp = delong(y, score, score2)
c1, c2 = pred == y, pred2 == y
tab = [[int(np.sum(c1 & c2)), int(np.sum(c1 & ~c2))], [int(np.sum(~c1 & c2)), int(np.sum(~c1 & ~c2))]]
pt, mt = calibration_curve(y, score, n_bins=10, strategy="uniform")
r = {
    "accuracy": M.accuracy_score(y, pred), "balanced_accuracy": M.balanced_accuracy_score(ym, pm),
    "precision": M.precision_score(y, pred), "recall": M.recall_score(y, pred), "f1": M.f1_score(y, pred),
    "f1_macro": M.f1_score(ym, pm, average="macro"), "f1_weighted": M.f1_score(ym, pm, average="weighted"),
    "precision_micro": M.precision_score(ym, pm, average="micro"), "fbeta2": M.fbeta_score(y, pred, beta=2),
    "mcc": M.matthews_corrcoef(ym, pm), "kappa": M.cohen_kappa_score(ym, pm),
    "confusion": M.confusion_matrix(ym, pm).tolist(),
    "roc_auc": M.roc_auc_score(y, score), "roc_auc_ovr": M.roc_auc_score(ym, probs, multi_class="ovr"),
    "ap": M.average_precision_score(y, score), "log_loss": M.log_loss(y, score), "log_loss_multi": M.log_loss(ym, probs),
    "brier": M.brier_score_loss(y, score), "top2": M.top_k_accuracy_score(ym, probs, k=2),
    "mae": M.mean_absolute_error(yr, pr), "mse": M.mean_squared_error(yr, pr), "r2": M.r2_score(yr, pr),
    "mape": M.mean_absolute_percentage_error(yr, pr), "median_ae": M.median_absolute_error(yr, pr),
    "max_error": M.max_error(yr, pr), "explained_variance": M.explained_variance_score(yr, pr),
    "ndcg": float(np.mean([ndcg_lin(q, len(q)) for q in rels])), "ndcg3": float(np.mean([ndcg_lin(q, 3) for q in rels])),
    "ndcg_sklearn_q0": float(M.ndcg_score([rels[0]], [list(range(len(rels[0]), 0, -1))])),
    "cal_frac_pos": pt.tolist(), "cal_mean_pred": mt.tolist(),
    "wasserstein": stats.wasserstein_distance(ref, cur), "psi": psi(ref, cur),
    "kl": float(stats.entropy(pdist, qdist)), "js": float(jensenshannon(pdist, qdist, base=2) ** 2),
    "mcnemar_exact_p": float(mcnemar(tab, exact=True).pvalue), "mcnemar_chi2_p": float(mcnemar(tab, exact=False, correction=True).pvalue),
    "auc2": M.roc_auc_score(y, score2), "delong_z": dz, "delong_p": dp,
}
data = {"y": y.tolist(), "score": score.tolist(), "score2": score2.tolist(), "pred": pred.tolist(), "pred2": pred2.tolist(),
        "ym": ym.tolist(), "pm": pm.tolist(), "probs": probs.tolist(), "yr": yr.tolist(), "pr": pr.tolist(), "rels": rels,
        "ref": ref.tolist(), "cur": cur.tolist(), "p": pdist, "q": qdist}
print(json.dumps({"data": data, "ref": {k: (float(v) if isinstance(v, (np.floating, float)) else v) for k, v in r.items()}}))
