# Regenerate examples/data/churn_preds.csv (synthetic model outputs, fixed seed)
import numpy as np, csv, sys
rng = np.random.default_rng(11)
w = csv.writer(sys.stdout)
w.writerow(["user_id", "month", "gender", "region", "tenure", "spend", "label", "score_old", "score_new"])
for i in range(3000):
    month = 1 if i < 1500 else 2
    g = rng.choice(["F", "M"])
    reg = rng.choice(["north", "south", "east", "west"], p=[0.3, 0.3, 0.25, 0.15])
    tenure = int(rng.integers(1, 60))
    spend = round(float(rng.gamma(2, 30 if month == 1 else 38)), 2)
    logit = -1.3 + 0.09 * (30 - tenure) - 0.012 * spend + (0.9 if reg == "west" else 0)
    p = 1 / (1 + np.exp(-logit))
    y = int(rng.random() < p)
    old = float(np.clip(p + rng.normal(0, 0.2) + (0.12 if g == "F" else -0.05), 0.001, 0.999))
    new = float(np.clip(p + rng.normal(0, 0.04), 0.001, 0.999))
    w.writerow([i + 1, month, g, reg, tenure, spend, y, round(old, 4), round(new, 4)])
