#!/usr/bin/env python3
"""Count explicit filter bypasses in Bash calls.

When a filter drops something the agent needed, the agent re-runs the command to
bypass the filter. The command then costs twice and both results enter context,
so a filter that mangles output costs more than no filter.

This counts explicit bypasses only (`rtk proxy`, `--no-filter`, `--raw`,
`RTK_DISABLE`). It does NOT pair a bypass to the filtered call it replaced.
Transcripts record the command the model wrote, not the PreToolUse hook rewrite,
so the filtered call is invisible. The count is therefore a floor: every bypass
here is real, and paired retries the hook rewrote are missed.

Usage:
  ./retries.py --files 60
"""
import argparse, glob, json, os, re
from collections import Counter

ROOT = os.path.expanduser("~/.claude/projects")
BYPASS = re.compile(r"\brtk\s+proxy\b|--no-filter\b|--raw\b|\bRTK_DISABLE")


def blocks(msg):
    c = (msg or {}).get("content")
    return c if isinstance(c, list) else []


def rlen(b):
    c = b.get("content")
    if isinstance(c, str):
        return len(c)
    if isinstance(c, list):
        return sum(len(x.get("text", "")) for x in c if isinstance(x, dict))
    return 0


def target_of(cmd):
    """Command the bypass actually runs, with the bypass wrapper removed."""
    c = re.sub(r"^\s*rtk\s+proxy\s+", "", cmd.strip())
    c = re.sub(r"^\s*[A-Za-z_][A-Za-z0-9_]*=\S*\s+", "", c)
    m = re.match(r"([\w./-]+)(?:\s+([\w-]+))?", c)
    if not m:
        return "(shell)"
    head = os.path.basename(m.group(1))
    sub = m.group(2) or ""
    if head in ("git", "cargo", "gh", "npm", "docker") and sub and not sub.startswith("-"):
        return f"{head} {sub}"
    return head


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--files", type=int, default=60)
    ap.add_argument("--top", type=int, default=15)
    a = ap.parse_args()

    paths = sorted(glob.glob(os.path.join(ROOT, "*", "*.jsonl")),
                   key=os.path.getmtime, reverse=True)[:a.files]

    total = bypass = 0
    bytes_total = bytes_bypass = 0
    by_target = Counter()
    bytes_by_target = Counter()

    for p in paths:
        pending = {}
        try:
            with open(p, errors="replace") as f:
                for line in f:
                    try:
                        d = json.loads(line)
                    except Exception:
                        continue
                    for b in blocks(d.get("message", {})):
                        if b.get("type") == "tool_use":
                            pending[b.get("id")] = (b.get("name"), b.get("input"))
                        elif b.get("type") == "tool_result":
                            name, inp = pending.pop(b.get("tool_use_id"), (None, None))
                            if name != "Bash":
                                continue
                            cmd = (inp or {}).get("command", "")
                            n = rlen(b)
                            total += 1
                            bytes_total += n
                            if BYPASS.search(cmd):
                                bypass += 1
                                bytes_bypass += n
                                t = target_of(cmd)
                                by_target[t] += 1
                                bytes_by_target[t] += n
        except Exception:
            continue

    if not total:
        print("no Bash calls found")
        return
    print(f"Bash calls scanned      : {total:>7}  ({bytes_total/1e6:.1f} MB)")
    print(f"explicit filter bypasses: {bypass:>7}  ({bytes_bypass/1e6:.1f} MB)")
    print(f"bypass rate             : {100*bypass/total:>6.1f}% of all Bash calls")
    print("  Floor, not ceiling — see module docstring.")
    if by_target:
        print(f"\n{'bypassed command':<20}{'calls':>8}{'MB':>9}{'% of bypass':>13}")
        for k, c in by_target.most_common(a.top):
            print(f"{k:<20}{c:>8}{bytes_by_target[k]/1e6:>9.2f}"
                  f"{100*c/bypass:>12.1f}%")


if __name__ == "__main__":
    main()
