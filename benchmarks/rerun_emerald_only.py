#!/usr/bin/env python3
"""Re-measure only the Emerald row of each benchmark, after the
`define_main` worker-pool-gating fix (crates/emerald-codegen/src/lib.rs),
and merge the new numbers into the existing raw_results.json in place —
every other language's row is left untouched, since this fix doesn't
touch their toolchains and re-measuring them again would just add noise
to already-good numbers. Same methodology as run_benchmarks.py: CPU time
via resource.getrusage(RUSAGE_CHILDREN) deltas, 10 runs, correctness-gated
before any timing counts. No cross-language alternation here since only
one language is being timed in this pass.
"""

import json
import pathlib
import resource
import statistics
import subprocess
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


def cpu_time_run(cmd):
  before = resource.getrusage(resource.RUSAGE_CHILDREN)
  r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
  after = resource.getrusage(resource.RUSAGE_CHILDREN)
  assert r.returncode == 0, f"{cmd} exited {r.returncode} during timing"
  return (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)


def main():
  results_path = BENCH / "raw_results.json"
  with open(results_path) as f:
    results = json.load(f)

  for bench, expected in BENCH_DEFS.items():
    bdir = BENCH / bench
    out_bin = TMP / f"{bench}_emerald_v2"
    ct = wall_compile([str(EMERALD_BIN), str(bdir / f"{bench}.em"), "-o", str(out_bin)])
    size = out_bin.stat().st_size

    r = subprocess.run([str(out_bin)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    assert r.returncode == 0, f"{bench}/Emerald exited {r.returncode}: {r.stderr}"
    assert r.stdout.strip() == expected, f"{bench}/Emerald output {r.stdout.strip()!r} != {expected!r}"

    runs = [cpu_time_run([str(out_bin)]) for _ in range(RUNS)]

    old = results[bench]["Emerald"]
    results[bench]["Emerald_before_fix"] = {k: old[k] for k in ("compile", "size", "runs", "mean", "min", "max", "stddev")}
    results[bench]["Emerald"] = {
      "bin": [str(out_bin)],
      "compile": ct,
      "size": size,
      "runs": runs,
      "mean": statistics.mean(runs),
      "min": min(runs),
      "max": max(runs),
      "stddev": statistics.stdev(runs) if len(runs) > 1 else 0.0,
    }
    print(f"{bench}: Emerald mean {results[bench]['Emerald']['mean']*1000:.3f} ms "
          f"(was {old['mean']*1000:.3f} ms), size {size} B (was {old['size']} B)")

  with open(results_path, "w") as f:
    json.dump(results, f, indent=2)
  print("WROTE benchmarks/raw_results.json (Emerald rows updated, old preserved as Emerald_before_fix)")


if __name__ == "__main__":
  main()
