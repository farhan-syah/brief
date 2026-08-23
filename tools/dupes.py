#!/usr/bin/env python3
"""Measure repeat cost: duplicate tool output and re-read amplification.

Two questions a folding filter cannot answer:
  1. How many tool bytes are literally the same content returned again?
  2. How many times is each result re-read as cached input on later turns?

Question 2 dominates cost. A result returned on turn 10 of a 100-turn session is
re-sent as input on every turn after it, so its true cost is size x turns-after.

Usage:
  ./dupes.py --files 60
"""
import argparse, glob, hashlib, json, os
from collections import defaultdict

ROOT = os.path.expanduser("~/.claude/projects")


def blocks(msg):
    c = (msg or {}).get("content")
    return c if isinstance(c, list) else []


def result_text(b):
    c = b.get("content")
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "".join(x.get("text", "") for x in c if isinstance(x, dict))
    return ""


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--files", type=int, default=60)
    a = ap.parse_args()

    paths = sorted(glob.glob(os.path.join(ROOT, "*", "*.jsonl")),
                   key=os.path.getmtime, reverse=True)[:a.files]

    tot = dup_bytes = dup_n = 0
    cmd_repeat_bytes = cmd_repeat_n = 0
    amp_num = amp_den = 0
    big = []

    for p in paths:
        pending, seen_out, seen_cmd = {}, {}, {}
        results = []          # (size, turn_index)
        turn = 0
        try:
            with open(p, errors="replace") as f:
                for line in f:
                    try:
                        d = json.loads(line)
                    except Exception:
                        continue
                    m = d.get("message", {})
                    if d.get("type") == "assistant":
                        turn += 1
                    for b in blocks(m):
                        t = b.get("type")
                        if t == "tool_use":
                            pending[b.get("id")] = (b.get("name"), b.get("input"))
                        elif t == "tool_result":
                            name, inp = pending.pop(b.get("tool_use_id"), (None, None))
                            if name != "Bash":
                                continue
                            txt = result_text(b)
                            n = len(txt)
                            tot += n
                            results.append((n, turn))
                            h = hashlib.blake2b(txt.encode(errors="replace"),
                                                digest_size=8).hexdigest()
                            if n > 40 and h in seen_out:
                                dup_bytes += n
                                dup_n += 1
                            seen_out[h] = True
                            c = (inp or {}).get("command", "")
                            if c and c in seen_cmd:
                                cmd_repeat_bytes += n
                                cmd_repeat_n += 1
                            seen_cmd[c] = True
        except Exception:
            continue
        if results:
            last = max(t for _, t in results)
            for n, t in results:
                amp_num += n * max(0, last - t)
                amp_den += n
            big.append((last, sum(n for n, _ in results)))

    if not tot:
        print("no Bash results found")
        return
    print(f"Bash result bytes scanned: {tot/1e6:.1f} MB\n")
    print(f"exact-duplicate output   : {dup_bytes/1e6:>6.2f} MB "
          f"({100*dup_bytes/tot:>5.1f}%)  {dup_n} calls")
    print(f"identical command re-run : {cmd_repeat_bytes/1e6:>6.2f} MB "
          f"({100*cmd_repeat_bytes/tot:>5.1f}%)  {cmd_repeat_n} calls")
    if amp_den:
        print(f"\nre-read amplification    : {amp_num/amp_den:>6.1f}x mean")
        print(f"  Every Bash byte is re-sent as cached input this many times "
              f"on later turns.")
        print(f"  Effective cached-input volume from Bash alone: "
              f"{amp_num/1e6:,.0f} MB")
    if big:
        big.sort(reverse=True)
        print(f"\nlongest sessions (turns, Bash MB):")
        for t, n in big[:5]:
            print(f"  {t:>5} turns   {n/1e6:>6.2f} MB")


if __name__ == "__main__":
    main()
