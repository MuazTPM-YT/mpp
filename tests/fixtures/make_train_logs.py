# Regenerate examples/data/train_run_a.jsonl and train_run_b.jsonl (synthetic, fixed seed)
import json, math, random
random.seed(1)

def run(path, nan_at=None, lr_peak=3e-4):
    with open(path, "w") as f:
        for step in range(10, 2010, 10):
            warm = 200
            lr = lr_peak * step / warm if step < warm else lr_peak * 0.5 * (1 + math.cos(math.pi * (step - warm) / 1800))
            loss = 2.2 * math.exp(-step / 500) + 0.9 + random.gauss(0, 0.015)
            grad = 1.0 + random.gauss(0, 0.08)
            tps = 41000 + random.gauss(0, 600)
            if step == 900:
                loss += 1.4
                grad = 25.0
            if 1300 <= step <= 1330:
                tps = 15000
            if nan_at and step >= nan_at:
                loss = float("nan")
                grad = float("inf")
            row = {"step": step, "loss": round(loss, 4), "lr": lr, "grad_norm": round(grad, 3), "tokens_per_sec": round(tps)}
            f.write(json.dumps(row) + "\n")
            if step % 100 == 0:
                val = 2.2 * math.exp(-step / 500) + 0.95 + max(0, step - 1400) * 0.0004 + random.gauss(0, 0.005)
                f.write(json.dumps({"step": step, "val_loss": round(val, 4)}) + "\n")

run("examples/data/train_run_a.jsonl")
run("examples/data/train_run_b.jsonl", nan_at=1210, lr_peak=1e-3)
