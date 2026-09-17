import json

d = json.load(open("benchmarks/raw_results.json"))
lines = []
for bench, langs in d.items():
  lines.append(f"== {bench} ==")
  for lang, info in langs.items():
    ct = f"{info['compile']*1000:.2f} ms" if info["compile"] is not None else "n/a"
    sz = f"{info['size']:,} B" if info["size"] is not None else "n/a"
    lines.append(
      f"{lang:8s} compile={ct:>12s} size={sz:>14s} "
      f"mean={info['mean']*1000:9.4f}ms min={info['min']*1000:9.4f}ms "
      f"max={info['max']*1000:9.4f}ms sd={info['stddev']*1000:8.5f}ms"
    )
out = "\n".join(lines) + "\n"
print(out)
with open("benchmarks/summary.txt", "w") as f:
  f.write(out)
