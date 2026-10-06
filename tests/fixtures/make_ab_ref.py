# Regenerate ab_ref.json: python make_ab_ref.py > ab_ref.json (needs scipy, numpy, statsmodels)
import json, numpy as np
from scipy import stats, integrate
from statsmodels.stats.proportion import proportions_ztest, confint_proportions_2indep
from statsmodels.stats.power import NormalIndPower

r = {}
# B=130/1000 vs A=100/1000
z, p = proportions_ztest([130, 100], [1000, 1000])
lo, hi = confint_proportions_2indep(130, 1000, 100, 1000, method="wald", compare="diff")
r["prop"] = {"z": float(z), "p": float(p), "ci": [float(lo), float(hi)]}
z, p = proportions_ztest([130, 100], [1000, 1000], alternative="larger")
r["prop_greater_p"] = float(p)
# power for means: effect size 0.2, n1 = 300, ratio 1.5
r["power_means"] = float(NormalIndPower().power(effect_size=0.2, nobs1=300, alpha=0.05, ratio=1.5))
r["power_means_one"] = float(NormalIndPower().power(effect_size=0.2, nobs1=300, alpha=0.05, ratio=1.0, alternative="larger"))
r["n_means"] = float(NormalIndPower().solve_power(effect_size=0.25, alpha=0.05, power=0.8, ratio=1.0))
# classic two-proportion sample size (Fleiss, no continuity), plus lower tail
p1, p2 = 0.10, 0.12
za, zb = stats.norm.ppf(0.975), stats.norm.ppf(0.8)
pb = (p1 + p2) / 2
r["n_props_formula"] = float(((za * np.sqrt(2 * pb * (1 - pb)) + zb * np.sqrt(p1 * (1 - p1) + p2 * (1 - p2))) / (p2 - p1)) ** 2)
# exact P(B > A) for Beta(1+30, 1+70) vs Beta(1+45, 1+55) by numeric integration
aa, ba, ab_, bb = 31, 71, 46, 56
f = lambda x: stats.beta.pdf(x, ab_, bb) * stats.beta.cdf(x, aa, ba)
r["prob_b_better"] = float(integrate.quad(f, 0, 1, epsabs=1e-13, epsrel=1e-12)[0])
print(json.dumps(r, indent=1))
