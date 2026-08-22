#!/usr/bin/env python3
"""代码总 LOC 统计 — Rust 源码（src 全部 + 根 build.rs）。

行分类（近似，按行统计）：
- 总行 / 空行 / 注释行（trim 后以 // 开头，含块注释 /* */ 区间内的行）
- 代码行 = 总行 - 空行 - 注释行（行内代码后附 `// 注释` 计入代码行）

用法: python tools/loc.py [路径...]   （默认: src + build.rs）
"""

import os
import sys
from pathlib import Path


def classify(text: str) -> tuple[int, int, int]:
    """返回 (空行, 注释行, 代码行)。简易块注释状态机（/* */ 可跨行）。"""
    blank = comment = code = 0
    in_block = False
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            blank += 1
            continue
        ct = 0  # 本行注释字符数（行内已消费）
        i = 0
        while i < len(line):
            two = line[i : i + 2]
            if in_block:
                ct += 1
                if two == "*/":
                    in_block = False
                    i += 2
                else:
                    i += 1
                continue
            if two == "//":
                ct += len(line) - i
                break
            if two == "/*":
                in_block = True
                ct += 2
                i += 2
                continue
            i += 1
        # 全行只剩注释字符（允许 /* 后未闭合到行尾）→ 注释行
        if ct >= len(line):
            comment += 1
        else:
            code += 1
    return blank, comment, code


def scan(paths: list[Path]) -> None:
    files = []
    for p in paths:
        if p.is_dir():
            files.extend(sorted(p.rglob("*.rs")))
        elif p.is_file():
            files.append(p)

    total = blank = comment = code = 0
    per_dir: dict[str, list[int]] = {}  # 目录 → [文件数, 代码行]
    print(f"{'文件':<58} {'总行':>6} {'空':>5} {'注释':>6} {'代码':>6}")
    print("-" * 85)
    for f in files:
        try:
            t = f.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            print(f"  [skip] {f}: {e}")
            continue
        b, c, co = classify(t)
        total += len(t.splitlines())
        blank += b
        comment += c
        code += co
        rel = os.path.relpath(f, Path.cwd()).replace("\\", "/")
        d = os.path.relpath(f.parent, Path.cwd()).replace("\\", "/")
        per_dir.setdefault(d, [0, 0])
        per_dir[d][0] += 1
        per_dir[d][1] += co
        print(f"{rel:<58} {len(t.splitlines()):>6} {b:>5} {c:>6} {co:>6}")

    print("-" * 85)
    print(f"{'TOTAL':<58} {total:>6} {blank:>5} {comment:>6} {code:>6}")
    print(f"\n按目录汇总（代码行）:")
    for d, (n, co) in sorted(per_dir.items(), key=lambda kv: -kv[1][1]):
        print(f"  {d:<50} {n:>4} 文件  {co:>6} 代码行")


if __name__ == "__main__":
    paths = [Path(p) for p in (sys.argv[1:] or ["./src", "./build.rs"])]
    scan(paths)
