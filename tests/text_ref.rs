// text metrics vs nltk BLEU, rouge-score, sacrebleu chrF (tests/fixtures/text_ref.json)
use mpp::stdlib::llm::text;

#[test]
fn text_metrics_match_reference_libraries() {
    let cases: serde_json::Value = serde_json::from_str(&std::fs::read_to_string("tests/fixtures/text_ref.json").unwrap()).unwrap();
    let mut bad = Vec::new();
    for c in cases.as_array().unwrap() {
        let cand = c["cand"].as_str().unwrap();
        let refs: Vec<String> = c["refs"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect();
        let got = [
            ("bleu", text::bleu(cand, &refs, 4, false)),
            ("bleu_smooth", text::bleu(cand, &refs, 4, true)),
            ("rouge1", text::rouge_n(cand, &refs[0], 1).2),
            ("rouge2", text::rouge_n(cand, &refs[0], 2).2),
            ("rougeL", text::rouge_l(cand, &refs[0]).2),
            ("chrf", text::chrf(cand, &refs, 6, 2.0)),
        ];
        for (k, g) in got {
            let w = c[k].as_f64().unwrap();
            if (g - w).abs() > 1e-9 * w.abs().max(1.0) {
                bad.push(format!("{k} {cand:?}: got {g}, want {w}"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
