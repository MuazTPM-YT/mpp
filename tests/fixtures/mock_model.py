# fake local model for tests: one JSON request per line in, one JSON reply per line out
import json, sys

for line in sys.stdin:
    req = json.loads(line)
    prompt = req.get("prompt", "")
    print("loading weights... (log lines are ignored)", flush=True)
    text = "Paris" if "France" in prompt else "echo: " + prompt
    reply = {"text": text, "tokens_in": len(prompt.split()), "tokens_out": len(text.split())}
    if req.get("logprobs"):
        reply["logprobs"] = [[w, -0.25] for w in text.split()]
    print(json.dumps(reply), flush=True)
