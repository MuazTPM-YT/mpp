# Regenerate stats_ref.json: python make_stats_ref.py > stats_ref.json (needs scipy, numpy)
import json, numpy as np
from scipy import stats

rng = np.random.default_rng(12345)
a = np.round(rng.normal(10, 2, 30), 4)
b = np.round(rng.normal(11, 2.5, 25), 4)
small_a = np.array([1.1, 2.3, 3.7, 4.2, 5.9, 6.1])
small_b = np.array([2.2, 4.4, 6.6, 7.7, 8.8, 9.9, 10.5])
ti = rng.integers(0, 6, 40).astype(float)
tj = rng.integers(1, 8, 35).astype(float)
px = np.round(rng.normal(5, 1, 20), 3)
py = np.round(px + rng.normal(0.3, 0.5, 20), 3)
g3 = np.round(rng.normal(12, 2, 18), 4)
big_a = np.round(rng.normal(0, 1, 150), 5)
big_b = np.round(rng.normal(0.2, 1.1, 140), 5)
shap = [np.round(rng.normal(0, 1, n), 5) for n in (3, 5, 8, 11, 20, 50, 300)]
shap.append(np.round(rng.exponential(1, 40), 5))
cont = [[12, 5, 9], [7, 14, 6]]
c22 = [[8, 2], [1, 5]]
pv = [0.01, 0.04, 0.03, 0.005, 0.2, 0.5]

def tt(r):
    return [float(r.statistic), float(r.pvalue)]

out = {"data": {k: list(map(float, v)) for k, v in dict(a=a, b=b, small_a=small_a, small_b=small_b, ti=ti, tj=tj, px=px, py=py, g3=g3, big_a=big_a, big_b=big_b).items()},
       "shapiro_data": [list(map(float, s)) for s in shap], "cont": cont, "c22": c22, "pv": pv}
r = {}
r["ttest_1samp"] = tt(stats.ttest_1samp(a, 9.5))
r["ttest_1samp_greater"] = tt(stats.ttest_1samp(a, 9.5, alternative="greater"))
r["welch"] = tt(stats.ttest_ind(a, b, equal_var=False))
r["student"] = tt(stats.ttest_ind(a, b, equal_var=True))
r["welch_less"] = tt(stats.ttest_ind(a, b, equal_var=False, alternative="less"))
r["welch_ci"] = list(map(float, stats.ttest_ind(a, b, equal_var=False).confidence_interval(0.95)))
r["paired"] = tt(stats.ttest_rel(px, py))
c = stats.chi2_contingency(cont)
r["chi2"] = [float(c.statistic), float(c.pvalue)]
c = stats.chi2_contingency(c22)
r["chi2_yates"] = [float(c.statistic), float(c.pvalue)]
r["chi2_gof"] = tt(stats.chisquare([18, 22, 30, 30], [25, 25, 25, 25]))
f = stats.fisher_exact(c22)
r["fisher"] = [float(f.statistic), float(f.pvalue)]
r["fisher_greater"] = [float(stats.fisher_exact(c22, alternative="greater").pvalue)]
r["mwu_asym"] = tt(stats.mannwhitneyu(a, b, method="asymptotic"))
r["mwu_ties"] = tt(stats.mannwhitneyu(ti, tj))
r["mwu_exact"] = tt(stats.mannwhitneyu(small_a, small_b))
r["mwu_exact_less"] = tt(stats.mannwhitneyu(small_a, small_b, alternative="less"))
r["wilcoxon_exact"] = tt(stats.wilcoxon(px, py))
r["wilcoxon_asym"] = tt(stats.wilcoxon(big_a[:100], big_b[:100]))
r["ks_exact"] = tt(stats.ks_2samp(a, b))
r["ks_small"] = tt(stats.ks_2samp(small_a, small_b))
r["ks_asym"] = tt(stats.ks_2samp(big_a, big_b, method="asymp"))
r["anova"] = tt(stats.f_oneway(a, b, g3))
r["kruskal"] = tt(stats.kruskal(ti, tj, g3))
r["levene"] = tt(stats.levene(a, b, g3))
r["levene_mean"] = tt(stats.levene(a, b, g3, center="mean"))
r["shapiro"] = [tt(stats.shapiro(s)) for s in shap]
pr = stats.pearsonr(px, py)
r["pearson"] = [float(pr.statistic), float(pr.pvalue)] + list(map(float, pr.confidence_interval(0.95)))
r["spearman"] = tt(stats.spearmanr(px, py))
r["spearman_ties"] = tt(stats.spearmanr(ti[:35], tj))
r["kendall_exact"] = tt(stats.kendalltau(px, py))
r["kendall_ties"] = tt(stats.kendalltau(ti[:35], tj))
lr = stats.linregress(px, py)
r["linregress"] = [lr.slope, lr.intercept, lr.rvalue, lr.pvalue, lr.stderr, lr.intercept_stderr]
r["skew"] = float(stats.skew(a)); r["kurtosis"] = float(stats.kurtosis(a))
from statsmodels.stats.multitest import multipletests
for m, k in [("bonferroni", "bonferroni"), ("holm", "holm"), ("fdr_bh", "bh"), ("fdr_by", "by"), ("simes-hochberg", "hochberg")]:
    r["adjust_" + k] = list(map(float, multipletests(pv, method=m)[1]))
import statsmodels.api as sm
X = sm.add_constant(np.column_stack([px, g3[:20] if len(g3) >= 20 else np.resize(g3, 20)]))
fit = sm.OLS(py, X).fit()
r["ols"] = {"params": list(map(float, fit.params)), "bse": list(map(float, fit.bse)), "pvalues": list(map(float, fit.pvalues)), "r2": float(fit.rsquared), "adj_r2": float(fit.rsquared_adj), "f": float(fit.fvalue), "f_p": float(fit.f_pvalue), "x2": list(map(float, np.resize(g3, 20)))}
r["dist"] = {"t_cdf": float(stats.t.cdf(-2.1, 7)), "t_ppf": float(stats.t.ppf(0.975, 12)), "chi2_sf": float(stats.chi2.sf(9.3, 4)),
             "f_sf": float(stats.f.sf(3.2, 2, 30)), "beta_ppf": float(stats.beta.ppf(0.3, 2.5, 7)), "binom_cdf": float(stats.binom.cdf(7, 20, 0.3)),
             "poisson_cdf": float(stats.poisson.cdf(4, 2.5)), "norm_ppf": float(stats.norm.ppf(0.001)), "kolmogorov_sf": [float(stats.kstwobign.sf(x)) for x in (0.2, 0.5, 1.0, 1.36, 2.0)]}
out["ref"] = r
print(json.dumps(out, indent=1))
