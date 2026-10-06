# Regenerate examples/data/checkout.csv (synthetic A/B data, fixed seed)
import numpy as np, csv, sys
rng = np.random.default_rng(7)
n = 4000
w = csv.writer(sys.stdout)
w.writerow(["user_id", "variant", "country", "day", "pre_revenue", "converted", "revenue"])
for i in range(n):
    v = "A" if rng.random() < 0.5 else "B"
    c = rng.choice(["US", "UK", "DE"], p=[0.5, 0.3, 0.2])
    day = int(rng.integers(1, 15))
    pre = round(float(rng.gamma(2.0, 15.0)), 2)
    base = 0.10 + (0.025 if v == "B" else 0) * (1.6 if day <= 4 else 1.0) + (0.02 if c == "US" else 0)
    conv = int(rng.random() < base)
    rev = round(pre * 0.6 + rng.normal(5 if v == "B" else 3, 4), 2) if conv else 0.0
    pre_s = "" if rng.random() < 0.01 else pre
    w.writerow([i + 1, v, c, day, pre_s, conv, rev])
