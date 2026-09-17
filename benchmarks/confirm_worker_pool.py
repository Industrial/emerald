import os
import resource
import subprocess

BIN = "/tmp/emerald_bench_work/sum_emerald"


def cpu_time(env=None):
  before = resource.getrusage(resource.RUSAGE_CHILDREN)
  subprocess.run([BIN], stdout=subprocess.DEVNULL, env=env)
  after = resource.getrusage(resource.RUSAGE_CHILDREN)
  return (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)


default_env = dict(os.environ)
one_worker_env = dict(os.environ)
one_worker_env["EMERALD_WORKERS"] = "1"

for label, env in [("default (32 workers)", default_env), ("EMERALD_WORKERS=1", one_worker_env)]:
  times = [cpu_time(env) for _ in range(5)]
  print(f"{label}: {[f'{t*1000:.3f}ms' for t in times]}")
