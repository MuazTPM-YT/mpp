# Regenerate text_ref.json: python make_text_ref.py > text_ref.json (needs nltk, rouge-score, sacrebleu)
import json
from nltk.translate.bleu_score import sentence_bleu, SmoothingFunction
from rouge_score import rouge_scorer
from sacrebleu.metrics import CHRF

pairs = [
    ("the cat sat on the mat", ["the cat is on the mat", "there is a cat on the mat"]),
    ("a quick brown fox jumps over the lazy dog", ["the quick brown fox jumped over the lazy dog"]),
    ("Paris is the capital of France.", ["The capital of France is Paris."]),
    ("hello world", ["goodbye moon"]),
    ("It is raining, and the streets are wet!", ["The streets are wet because it is raining."]),
]
sc = rouge_scorer.RougeScorer(["rouge1", "rouge2", "rougeL"], use_stemmer=False)
chrf = CHRF()
out = []
for cand, refs in pairs:
    r = sc.score(refs[0], cand)
    out.append({
        "cand": cand, "refs": refs,
        "bleu": sentence_bleu([x.split() for x in refs], cand.split()),
        "bleu_smooth": sentence_bleu([x.split() for x in refs], cand.split(), smoothing_function=SmoothingFunction().method1),
        "rouge1": r["rouge1"].fmeasure, "rouge2": r["rouge2"].fmeasure, "rougeL": r["rougeL"].fmeasure,
        "chrf": chrf.sentence_score(cand, refs).score,
    })
print(json.dumps(out, indent=1))
