#!/usr/bin/env python3
"""thread_sample.txt → CSV + pandas 分析。

解析 GI-Utils「线程采样」功能的输出表（固定列空格分隔，多段追加），
写出 thread_sample.csv（与输入同目录），并打印分析摘要：
热度 Top、CPU 占比集中度、按起始地址聚合的线程层、句柄列填充率。

用法: python thread_sample_analyzer.py [thread_sample.txt 路径]
"""

import re
import sys
from pathlib import Path

import pandas as pd

SECTION_RE = re.compile(
    r"═+\s+YuanShen\.exe \(PID (\d+)\) (\S+)\s+采样窗口 (\d+)ms\s+"
    r"NT快照(可用|不可用)\s+线程 (\d+)\s+═+"
)

COLUMNS = [
    "tid", "name", "state", "wait", "cpu_pct", "user_ms", "kernel_ms",
    "cpu_time_s", "cycles_delta", "cycles_total", "ctx_switches",
    "base_pri", "dyn_pri", "ideal_cpu", "suspend", "mem_pri", "io_pri",
    "start", "started",
]


def parse_cpu_time(s: str) -> float:
    """'0h41m25.7s' → 秒。"""
    if s == "-":
        return float("nan")
    m = re.match(r"(\d+)h(\d+)m([\d.]+)s", s)
    if not m:
        return float("nan")
    h, mi, sec = m.groups()
    return int(h) * 3600 + int(mi) * 60 + float(sec)


def parse_txt(path: Path) -> pd.DataFrame:
    rows, meta = [], None
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.rstrip()
        if not line:
            continue
        m = SECTION_RE.search(line)
        if m:
            meta = {
                "pid": int(m.group(1)),
                "sampled_at": m.group(2),
                "nt_ok": m.group(4) == "可用",
                "threads_declared": int(m.group(5)),
            }
            continue
        if line.startswith("TID "):  # 列头
            continue
        tokens = line.split()
        if meta is None or len(tokens) < len(COLUMNS):
            continue
        # NAME 可能含空格：末尾 17 个定长字段之外的中段全部归并进 name
        n = len(tokens)
        head, tail = tokens[:1], tokens[n - 17 :]
        name = " ".join(tokens[1 : n - 17])
        vals = head + [name] + tail
        rows.append({**meta, **dict(zip(COLUMNS, vals))})

    df = pd.DataFrame(rows)
    # 类型清洗：'-' → NaN，数值列转型
    num_int = ["tid", "user_ms", "kernel_ms", "cycles_delta", "cycles_total",
               "ctx_switches", "base_pri", "dyn_pri", "ideal_cpu", "suspend",
               "mem_pri", "io_pri"]
    for c in num_int:
        df[c] = pd.to_numeric(df[c].replace("-", ""), errors="coerce").astype("Int64")
    df["cpu_pct"] = pd.to_numeric(df["cpu_pct"], errors="coerce")
    df["cpu_time_s"] = df["cpu_time_s"].map(parse_cpu_time)
    return df


def main() -> None:
    txt = Path(sys.argv[1] if len(sys.argv) > 1 else "thread_sample.txt")
    df = parse_txt(txt)
    if df.empty:
        sys.exit(f"no data rows parsed from {txt}")

    csv_path = txt.with_suffix(".csv")
    df.to_csv(csv_path, index=False)
    print(f"CSV 写出: {csv_path}（{len(df)} 行）\n")

    for (pid, at), g in df.groupby(["pid", "sampled_at"], sort=False):
        print(f"══ PID {pid} @ {at} — {len(g)} 线程 ══")
        handles = g["cycles_total"].notna().sum()
        print(f"句柄列填充: {handles}/{len(g)}"
              f"（{'pinning 查询侧绿灯' if handles == len(g) else '存在被拒线程'}）")

        total_cpu = g["cpu_pct"].sum()
        top = g.sort_values(["cycles_delta", "cpu_pct"], ascending=False)
        print("\n── 热度 Top 10（按 Cycles Delta）──")
        cols = ["tid", "cpu_pct", "cycles_delta", "ideal_cpu", "dyn_pri",
                "ctx_switches", "start"]
        print(top[cols].head(10).to_string(index=False))

        print("\n── CPU 集中度 ──")
        for k in (1, 2, 5, 10):
            share = top["cpu_pct"].head(k).sum() / total_cpu * 100
            print(f"Top{k:>2}: {top['cpu_pct'].head(k).sum():6.1f}% / "
                  f"总 {total_cpu:6.1f}%（{share:5.1f}% 集中度）")

        print("\n── 按起始地址聚合（线程层）──")
        agg = (g.groupby("start")
                 .agg(threads=("tid", "count"),
                      cpu_sum=("cpu_pct", "sum"),
                      cpu_max=("cpu_pct", "max"),
                      ideal_range=("ideal_cpu", lambda s: f"{s.min():.0f}-{s.max():.0f}"))
                 .sort_values("cpu_sum", ascending=False))
        print(agg.head(12).to_string())

        busy = top[top["cpu_pct"] > 1]
        print("\n── 忙线程（CPU>1%）Ideal CPU 分布 ──")
        print(busy["ideal_cpu"].value_counts().sort_index().to_string())
        print()


if __name__ == "__main__":
    main()
