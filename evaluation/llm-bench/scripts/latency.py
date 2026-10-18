#!/usr/bin/env python3
"""Real llama-server latency for Badi-style autocomplete.

Starts one llama-server, measures load time and peak RSS, then runs the
scenarios from the brief at each prompt size and writes one JSON line per
request. Every cache claim comes from the server's own counters (timings.cache_n,
timings.prompt_n), never from speed. Hybrid/recurrent correctness is checked by
re-running the final prompt of each scenario with cache_prompt=false and
comparing the greedy text.

Usage:
  latency.py --server BIN --model GGUF --label NAME --out results/latency/NAME.jsonl
             [--threads 6] [--threads-batch 6] [--ngl 0] [--sizes 128,512,1024,2000]
             [--langs en,de] [--typing 20] [--extra "--flag value"]
"""

from __future__ import annotations

import argparse
import http.client
import json
import os
import random
import shlex
import signal
import socket
import subprocess
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MIN_FREE_BYTES = 2 * 1024**3
N_PREDICT = 16


def mem_available() -> int:
    for line in open("/proc/meminfo"):
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) * 1024
    raise RuntimeError("MemAvailable missing")


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Server:
    """One llama-server child, its peak RSS and a memory guard."""

    def __init__(self, binary: str, model: str, args: list[str], log_path: Path):
        size = os.path.getsize(model)
        # Weights plus a working allowance must leave the 2 GiB floor intact.
        need = size + 768 * 1024**2
        if mem_available() - need < MIN_FREE_BYTES:
            raise SystemExit(f"refusing to load: {mem_available()/1e9:.2f} GB available, "
                             f"{need/1e9:.2f} GB needed, 2 GiB floor")
        self.port = free_port()
        self.log = open(log_path, "w")
        cmd = [binary, "-m", model, "--host", "127.0.0.1", "--port", str(self.port), *args]
        self.cmd = cmd
        self.t_spawn = time.monotonic()
        self.proc = subprocess.Popen(cmd, stdout=self.log, stderr=subprocess.STDOUT,
                                     start_new_session=True)
        self.peak_rss = 0
        self.min_available = mem_available()
        self.guard_tripped = False
        self._stop = threading.Event()
        self._watch = threading.Thread(target=self._watch_memory, daemon=True)
        self._watch.start()
        self.load_s = self._wait_ready()

    def _watch_memory(self) -> None:
        status = f"/proc/{self.proc.pid}/status"
        while not self._stop.wait(0.05):
            try:
                for line in open(status):
                    if line.startswith("VmHWM:"):
                        self.peak_rss = max(self.peak_rss, int(line.split()[1]) * 1024)
            except FileNotFoundError:
                return
            avail = mem_available()
            self.min_available = min(self.min_available, avail)
            if avail < MIN_FREE_BYTES:
                self.guard_tripped = True
                self.proc.send_signal(signal.SIGKILL)
                return

    def _wait_ready(self) -> float:
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise SystemExit(f"llama-server exited {self.proc.returncode}; see {self.log.name}")
            try:
                if self.request("GET", "/health")[0] == 200:
                    return time.monotonic() - self.t_spawn
            except OSError:
                pass
            time.sleep(0.05)
        raise SystemExit("llama-server not ready in 600 s")

    def request(self, method: str, path: str, body: dict | None = None):
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=600)
        try:
            conn.request(method, path, body=json.dumps(body) if body is not None else None,
                         headers={"Content-Type": "application/json"})
            resp = conn.getresponse()
            data = resp.read()
            return resp.status, (json.loads(data) if data else None)
        finally:
            conn.close()

    def stop(self) -> None:
        self._stop.set()
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(20)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self.log.close()


def complete(server: Server, prompt: str, *, cache: bool = True, n_predict: int = N_PREDICT,
             cancel_after_first: bool = False, n_probs: int = 0) -> dict:
    """One streamed /completion. Times are monotonic, from just before send."""
    body = {"prompt": prompt, "n_predict": n_predict, "temperature": 0.0, "seed": 42,
            "stream": True, "cache_prompt": cache, "timings_per_token": True,
            "ignore_eos": False}
    if n_probs:
        body["n_probs"] = n_probs
    conn = http.client.HTTPConnection("127.0.0.1", server.port, timeout=600)
    t0 = time.monotonic()
    conn.request("POST", "/completion", body=json.dumps(body),
                 headers={"Content-Type": "application/json"})
    resp = conn.getresponse()
    if resp.status != 200:
        raise RuntimeError(f"/completion {resp.status}: {resp.read()[:300]!r}")
    ttft = None
    text = []
    last = None
    first_timings = None
    probs = []
    while True:
        line = resp.readline()
        if not line:
            break
        line = line.strip()
        if not line.startswith(b"data: "):
            continue
        chunk = json.loads(line[6:])
        if chunk.get("content") and ttft is None:
            ttft = time.monotonic() - t0
            first_timings = chunk.get("timings")
            if cancel_after_first:
                text.append(chunk["content"])
                conn.close()  # dropping the connection is how Badi cancels
                return {"ttft_s": ttft, "total_s": time.monotonic() - t0, "text": "".join(text),
                        "cancelled": True, "timings": first_timings}
        text.append(chunk.get("content", ""))
        probs.extend(chunk.get("completion_probabilities") or [])
        last = chunk
        if chunk.get("stop"):
            break
    total = time.monotonic() - t0
    conn.close()
    timings = (last or {}).get("timings") or {}
    return {"ttft_s": ttft, "total_s": total, "text": "".join(text), "cancelled": False,
            "timings": timings, "first_timings": first_timings,
            "tokens_cached": (last or {}).get("tokens_cached"),
            "tokens_evaluated": (last or {}).get("tokens_evaluated"),
            "stop_type": (last or {}).get("stop_type"), "probs": probs}


class Doc:
    """A disposable synthetic document cut at exact token counts of this model."""

    def __init__(self, server: Server, lang: str, seed: int, min_tokens: int):
        sentences = (ROOT / "corpus" / f"{lang}.txt").read_text().strip().split("\n")
        rng = random.Random(seed)
        header = {"en": f"Note {seed}: ", "de": f"Notiz {seed}: "}[lang]
        parts, n = [header], 0
        while n < min_tokens + 400:
            batch = sentences[:]
            rng.shuffle(batch)
            parts.extend(s + " " for s in batch)
            n = len(self.tokenize(server, "".join(parts)))
        self.text = "".join(parts)
        self.tokens = self.tokenize(server, self.text)
        self.server = server

    @staticmethod
    def tokenize(server: Server, text: str) -> list[int]:
        status, body = server.request("POST", "/tokenize", {"content": text, "add_special": False})
        if status != 200 or not body:
            raise RuntimeError(f"/tokenize {status}")
        return body["tokens"]

    def prefix(self, n: int) -> str:
        status, body = self.server.request("POST", "/detokenize", {"tokens": self.tokens[:n]})
        if status != 200 or not body:
            raise RuntimeError(f"/detokenize {status}")
        return body["content"]


def record(out, base: dict, scenario: str, step: int, prompt: str, result: dict,
           expected_prompt_tokens: int | None = None) -> dict:
    t = result.get("timings") or {}
    row = {**base, "scenario": scenario, "step": step, "prompt_chars": len(prompt),
           "ttft_ms": None if result["ttft_s"] is None else round(result["ttft_s"] * 1000, 2),
           "total_ms": round(result["total_s"] * 1000, 2),
           "prompt_n": t.get("prompt_n"), "cache_n": t.get("cache_n"),
           "prompt_ms": t.get("prompt_ms"), "predicted_n": t.get("predicted_n"),
           "predicted_ms": t.get("predicted_ms"),
           "predicted_per_second": t.get("predicted_per_second"),
           "prompt_per_second": t.get("prompt_per_second"),
           "tokens_cached": result.get("tokens_cached"),
           "tokens_evaluated": result.get("tokens_evaluated"),
           "cancelled": result["cancelled"], "text": result["text"],
           "expected_prompt_tokens": expected_prompt_tokens}
    out.write(json.dumps(row, ensure_ascii=False) + "\n")
    out.flush()
    return row


def divergence(ref: dict, cached_text: str) -> dict | None:
    """Where the cached text leaves the uncached greedy path, and the uncached top-5 there.

    A benign numeric tie-flip shows the cached token among the uncached runner-ups with a
    small probability gap; stale context tends to leave the top-5 entirely."""
    if ref["text"] == cached_text:
        return None
    pos = 0
    for i, step in enumerate(ref["probs"]):
        tok = step.get("token", "")
        if cached_text[pos:pos + len(tok)] != tok:
            tops = [(t.get("token"), round(2.718281828 ** t["logprob"], 4))
                    for t in step.get("top_logprobs", []) if "logprob" in t]
            rest = cached_text[pos:]
            alt = next(((t, p) for t, p in tops[1:] if t and rest.startswith(t)), None)
            gap = round(tops[0][1] - alt[1], 4) if alt and tops else None
            return {"token_index": i, "top": tops, "cached_next": rest[:12],
                    "cached_in_top5": alt is not None, "prob_gap": gap,
                    "class": "tie_flip" if gap is not None and gap < 0.2 else "suspect"}
        pos += len(tok)
    return {"token_index": len(ref["probs"]), "class": "length_only"}


def check(server: Server, out, base: dict, scenario: str, prompt: str, cached: dict) -> None:
    """Re-run the same prompt with no cache; greedy text must match."""
    ref = complete(server, prompt, cache=False, n_probs=5)
    row = {**base, "scenario": scenario + ":uncached_reference", "step": -1,
           "ttft_ms": round(ref["ttft_s"] * 1000, 2) if ref["ttft_s"] else None,
           "prompt_n": (ref["timings"] or {}).get("prompt_n"),
           "cache_n": (ref["timings"] or {}).get("cache_n"),
           "text": ref["text"], "cached_text": cached["text"],
           "text_match": ref["text"] == cached["text"],
           "divergence": divergence(ref, cached["text"])}
    out.write(json.dumps(row, ensure_ascii=False) + "\n")
    out.flush()


def run(args) -> None:
    sizes = [int(s) for s in args.sizes.split(",")]
    langs = args.langs.split(",")
    # The default 8 GiB host prompt cache could break the 2 GiB free-RAM floor.
    server_args = ["-c", str(args.ctx), "-np", "1", "--cache-reuse", str(args.cache_reuse),
                   "--cache-ram", str(args.cache_ram),
                   "-t", str(args.threads), "-tb", str(args.threads_batch or args.threads),
                   "-ngl", str(args.ngl), "--no-webui", *shlex.split(args.extra)]
    log_path = Path(args.out).with_suffix(".server.log")
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    server = Server(args.server, args.model, server_args, log_path)
    base = {"label": args.label, "model": os.path.basename(args.model),
            "threads": args.threads, "threads_batch": args.threads_batch or args.threads,
            "ngl": args.ngl, "ctx": args.ctx, "cache_reuse": args.cache_reuse, "cache_ram": args.cache_ram,
            "extra": args.extra}
    meta = {**base, "scenario": "server", "cmd": server.cmd, "load_s": round(server.load_s, 3),
            "mem_available_before": mem_available()}
    with open(args.out, "w") as out:
        try:
            # First use after load: includes any lazy init (e.g. Vulkan pipelines).
            first = complete(server, "Hello", cache=False, n_predict=4)
            meta["first_request_ms"] = round(first["total_s"] * 1000, 2)
            for lang in langs:
                for size in sizes:
                    if size + N_PREDICT + 8 > args.ctx:
                        continue
                    b = {**base, "lang": lang, "size": size}
                    seed = 1000 * sizes.index(size) + (0 if lang == "en" else 500)
                    doc = Doc(server, lang, seed, size + 200)
                    other = Doc(server, lang, seed + 1, size + 200)

                    # a. cold: a document whose first tokens nothing in cache shares.
                    p = doc.prefix(size)
                    r = complete(server, p)
                    record(out, b, "a_cold", 0, p, r, size)

                    # b. typing: append 1-5 tokens, args.typing times.
                    rng = random.Random(seed)
                    n = size
                    for step in range(args.typing):
                        n += rng.randint(1, 5)
                        p = doc.prefix(n)
                        r = complete(server, p)
                        record(out, b, "b_typing", step, p, r, n)
                    check(server, out, b, "b_typing", p, r)

                    # c. reject: backspace 10-30 tokens, type something else.
                    complete(server, doc.prefix(n))  # restore the warm state check() replaced
                    for step in range(3):
                        back = rng.randint(10, 30)
                        kept = doc.prefix(n - back)
                        p = kept + other.prefix(rng.randint(3, 8)).split(": ", 1)[-1]
                        r = complete(server, p)
                        record(out, b, "c_reject_retype", step, p, r)
                    check(server, out, b, "c_reject_retype", p, r)

                    # d. edit early: change one word about 10% into the document.
                    complete(server, doc.prefix(n))
                    full = doc.prefix(n)
                    cut = max(8, int(len(full) * 0.1))
                    space = full.find(" ", cut)
                    p = full[:space] + " very" + full[space:]
                    r = complete(server, p)
                    record(out, b, "d_edit_early", 0, p, r)
                    check(server, out, b, "d_edit_early", p, r)

                    # e. cancel mid-flight, then the replacement with two more tokens.
                    complete(server, doc.prefix(n))
                    p1 = doc.prefix(n + 2)
                    r1 = complete(server, p1, n_predict=64, cancel_after_first=True)
                    record(out, b, "e_cancelled", 0, p1, r1)
                    p2 = doc.prefix(n + 4)
                    r2 = complete(server, p2)
                    record(out, b, "e_replacement", 0, p2, r2, n + 4)
                    check(server, out, b, "e_replacement", p2, r2)

                    # f. switch documents: other doc, then back to this one.
                    po = other.prefix(size)
                    complete(server, po)
                    pd = doc.prefix(n + 4)
                    r = complete(server, pd)
                    record(out, b, "f_switch_back", 0, pd, r, n + 4)
                    r = complete(server, po + " ")
                    record(out, b, "f_switch_other", 0, po + " ", r)
                    check(server, out, b, "f_switch_other", po + " ", r)
        finally:
            meta["peak_rss_bytes"] = server.peak_rss
            meta["min_mem_available"] = server.min_available
            meta["memory_guard_tripped"] = server.guard_tripped
            server.stop()
            out.write(json.dumps(meta) + "\n")
    print(json.dumps({k: meta[k] for k in ("label", "load_s", "first_request_ms",
                                           "peak_rss_bytes", "memory_guard_tripped")
                      if k in meta}))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--threads", type=int, default=6)
    ap.add_argument("--threads-batch", type=int)
    ap.add_argument("--ngl", type=int, default=0)
    ap.add_argument("--ctx", type=int, default=2048)
    ap.add_argument("--cache-reuse", type=int, default=256)
    ap.add_argument("--cache-ram", type=int, default=1024)
    ap.add_argument("--sizes", default="128,512,1024,2000")
    ap.add_argument("--langs", default="en,de")
    ap.add_argument("--typing", type=int, default=20)
    ap.add_argument("--extra", default="")
    run(ap.parse_args())


if __name__ == "__main__":
    main()
