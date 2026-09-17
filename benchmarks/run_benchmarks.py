#!/usr/bin/env python3
"""Rigorous multi-run, multi-language benchmark driver.

Requires ruby, crystal, rustc, cc, g++, python3, and the emerald-cli
binary (built at ../target/debug/emerald) all on PATH. Intended to be
invoked inside one `nix-shell -p ruby crystal python3` session so ruby/
crystal don't pay repeated nix-shell startup overhead.

For each benchmark: compiles every language variant once, verifies
every variant's output against the expected value (a wrong-but-fast
program never gets a timing recorded), then times each variant 10
times, alternating language order each round to spread out any
systematic drift (thermal throttling, background load). CPU time
(user+sys) is measured via resource.getrusage(RUSAGE_CHILDREN) deltas,
not wall clock, to exclude scheduler noise.
"""

import json
import pathlib
import resource
import statistics
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
BENCH = ROOT / "benchmarks"
TMP = pathlib.Path("/tmp/emerald_bench_work")
TMP.mkdir(exist_ok=True)
EMERALD_BIN = ROOT / "target/debug/emerald"
RUNS = 10

BENCH_DEFS = {
  "sum": "49999995000000",
  "array_traversal": "210000000",
  "sum_of_squares": "333332833333500000",
  "fibonacci": "832040",
  "object_allocation": "499999500000",
  "method_dispatch": "50000005000000",
}


def wall_compile(cmd):
  t0 = time.perf_counter()
  r = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
  t1 = time.perf_counter()
  if r.returncode != 0:
    raise RuntimeError(f"compile failed: {cmd}\n{r.stderr}")
  return t1 - t0


def capture(cmd):
  return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


def cpu_time_run(cmd):
  before = resource.getrusage(resource.RUSAGE_CHILDREN)
  r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
  after = resource.getrusage(resource.RUSAGE_CHILDREN)
  assert r.returncode == 0, f"{cmd} exited {r.returncode} during timing"
  return (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)


def main():
  results_path = BENCH / "raw_results.json"
  results = {}
  if results_path.exists():
    with open(results_path) as f:
      results = json.load(f)
    print(f"resuming, already have: {list(results.keys())}", file=sys.stderr)
  for bench, expected in BENCH_DEFS.items():
    if bench in results:
      continue
    bdir = BENCH / bench
    langs = {}

    ct = wall_compile([str(EMERALD_BIN), str(bdir / f"{bench}.em"), "-o", str(TMP / f"{bench}_emerald")])
    langs["Emerald"] = {"bin": [str(TMP / f"{bench}_emerald")], "compile": ct, "size": (TMP / f"{bench}_emerald").stat().st_size}

    ct = wall_compile(["rustc", "-O", str(bdir / f"{bench}.rs"), "-o", str(TMP / f"{bench}_rust")])
    langs["Rust"] = {"bin": [str(TMP / f"{bench}_rust")], "compile": ct, "size": (TMP / f"{bench}_rust").stat().st_size}

    ct = wall_compile(["cc", "-O2", str(bdir / f"{bench}.c"), "-o", str(TMP / f"{bench}_c")])
    langs["C"] = {"bin": [str(TMP / f"{bench}_c")], "compile": ct, "size": (TMP / f"{bench}_c").stat().st_size}

    ct = wall_compile(["g++", "-O2", str(bdir / f"{bench}.cpp"), "-o", str(TMP / f"{bench}_cpp")])
    langs["C++"] = {"bin": [str(TMP / f"{bench}_cpp")], "compile": ct, "size": (TMP / f"{bench}_cpp").stat().st_size}

    ct = wall_compile(["crystal", "build", "--release", str(bdir / f"{bench}.cr"), "-o", str(TMP / f"{bench}_crystal")])
    langs["Crystal"] = {"bin": [str(TMP / f"{bench}_crystal")], "compile": ct, "size": (TMP / f"{bench}_crystal").stat().st_size}

    langs["Ruby"] = {"bin": ["ruby", str(bdir / f"{bench}.rb")], "compile": None, "size": None}

    for lang, info in langs.items():
      r = capture(info["bin"])
      out = r.stdout.strip()
      assert r.returncode == 0, f"{bench}/{lang} exited {r.returncode}: {r.stderr}"
      assert out == expected, f"{bench}/{lang} output {out!r} != expected {expected!r}"

    names = list(langs.keys())
    timings = {n: [] for n in names}
    for round_i in range(RUNS):
      order = names if round_i % 2 == 0 else list(reversed(names))
      for lang in order:
        timings[lang].append(cpu_time_run(langs[lang]["bin"]))

    for lang in names:
      ts = timings[lang]
      langs[lang]["runs"] = ts
      langs[lang]["mean"] = statistics.mean(ts)
      langs[lang]["min"] = min(ts)
      langs[lang]["max"] = max(ts)
      langs[lang]["stddev"] = statistics.stdev(ts) if len(ts) > 1 else 0.0

    results[bench] = langs
    with open(BENCH / "raw_results.json", "w") as f:
      json.dump(results, f, indent=2)
    print(f"done: {bench}", file=sys.stderr)

  print("WROTE benchmarks/raw_results.json", file=sys.stderr)


if __name__ == "__main__":
  main()
