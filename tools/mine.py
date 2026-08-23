#!/usr/bin/env python3
"""Mine Claude Code transcripts for where tool-result bytes actually go.

Pairs each tool_use with its tool_result, then buckets result size by tool and,
for Bash, by the leading command word. Output tells you which filters are worth
building and which volume no hook can reach.

Only PreToolUse on Bash can rewrite a tool call, so Read/Grep/Glob volume is
reported separately as the unreachable ceiling.

Usage:
  ./mine.py                      # 40 most recent transcripts
  ./mine.py --files 200 --top 30
"""
import argparse, json, os, glob, re
from collections import defaultdict

ROOT = os.path.expanduser("~/.claude/projects")


def blocks(msg):
    c = (msg or {}).get("content")
    return c if isinstance(c, list) else []


SUBCMD = {"git", "cargo", "npm", "pnpm", "yarn", "docker", "pacman", "systemctl",
          "uv", "rtk", "gh", "kubectl", "ollama", "pip", "pytest"}
WRAPPER = {"sudo", "time", "nohup", "nice", "xargs", "command", "exec", "then",
           "do", "done", "fi", "else", "elif"}


def cmd_key(inp):
    """Identify the command that produced the output.

    A raw first-word split buckets `cd x && cargo test` under `cd`, which hid
    22% of Bash volume behind a no-op. Strip shell scaffolding first, then take
    the last stage of a pipeline, since that stage is what reaches the model.
    """
    c = (inp or {}).get("command", "").strip()
    if not c:
        return "(empty)"
    # Last pipeline stage decides what the model actually sees.
    depth, cut = 0, 0
    for i, ch in enumerate(c):
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        elif ch == "|" and depth == 0 and c[i:i + 2] != "||" and c[i - 1:i] != "|":
            cut = i + 1
    seg = c[cut:] if cut else c
    # Drop leading `cd path &&`, `VAR=val`, and wrapper words.
    for _ in range(6):
        s = seg.strip()
        s2 = re.sub(r"^cd\s+[^&;|]+(&&|;)\s*", "", s)
        s2 = re.sub(r"^[A-Za-z_][A-Za-z0-9_]*=(\"[^\"]*\"|'[^']*'|\S*)\s+", "", s2)
        s2 = re.sub(r"^(%s)\s+" % "|".join(WRAPPER), "", s2)
        s2 = re.sub(r"^[({]\s*", "", s2)
        if s2 == s:
            break
        seg = s2
    m = re.match(r"([\w./-]+)(?:\s+(-{0,2}[\w-]+))?", seg.strip())
    if not m:
        return "(shell)"
    head = os.path.basename(m.group(1))
    sub = m.group(2) or ""
    if head in SUBCMD and sub and not sub.startswith("-"):
        return f"{head} {sub}"
    return head


def result_len(b):
    c = b.get("content")
    if isinstance(c, str):
        return len(c)
    if isinstance(c, list):
        return sum(len(x.get("text", "")) for x in c if isinstance(x, dict))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--files", type=int, default=40)
    ap.add_argument("--top", type=int, default=25)
    a = ap.parse_args()

    paths = sorted(glob.glob(os.path.join(ROOT, "*", "*.jsonl")),
                   key=os.path.getmtime, reverse=True)[:a.files]
    by_tool = defaultdict(lambda: [0, 0])   # tool -> [bytes, calls]
    by_cmd = defaultdict(lambda: [0, 0])
    sizes = []
    scanned = 0

    for p in paths:
        pending = {}
        try:
            with open(p, errors="replace") as f:
                for line in f:
                    try:
                        d = json.loads(line)
                    except Exception:
                        continue
                    m = d.get("message", {})
                    for b in blocks(m):
                        t = b.get("type")
                        if t == "tool_use":
                            pending[b.get("id")] = (b.get("name"), b.get("input"))
                        elif t == "tool_result":
                            name, inp = pending.pop(b.get("tool_use_id"), (None, None))
                            if not name:
                                continue
                            n = result_len(b)
                            by_tool[name][0] += n
                            by_tool[name][1] += 1
                            if name == "Bash":
                                sizes.append(n)
                                k = cmd_key(inp)
                                by_cmd[k][0] += n
                                by_cmd[k][1] += 1
        except Exception:
            continue
        scanned += 1

    total = sum(v[0] for v in by_tool.values())
    if not total:
        print("no tool results found")
        return
    print(f"scanned {scanned} transcripts   total tool-result bytes: {total/1e6:.1f} MB")
    print(f"(~{total/4/1e6:.1f}M tokens at 4 bytes/token)\n")

    print(f"{'tool':<16}{'MB':>9}{'share':>8}{'calls':>8}{'avg B':>9}{'hookable':>10}")
    for k, (n, c) in sorted(by_tool.items(), key=lambda x: -x[1][0])[:a.top]:
        hook = "yes" if k == "Bash" else "NO"
        print(f"{k:<16}{n/1e6:>9.1f}{100*n/total:>7.1f}%{c:>8}{n//max(c,1):>9}{hook:>10}")

    if sizes:
        sizes.sort(reverse=True)
        tot = sum(sizes)
        print(f"\nBash call-size distribution ({len(sizes)} calls)")
        for pct in (0.1, 1, 5, 10, 25, 50):
            k = max(1, int(len(sizes) * pct / 100))
            print(f"  top {pct:>4}% of calls ({k:>5}) hold "
                  f"{100*sum(sizes[:k])/tot:>5.1f}% of Bash bytes  "
                  f"(>= {sizes[k-1]:,} B)")
        print(f"  median call: {sizes[len(sizes)//2]:,} B   "
              f"max: {sizes[0]:,} B")
        over = [s for s in sizes if s > 20000]
        print(f"  calls over 20 KB: {len(over)} "
              f"({100*sum(over)/tot:.1f}% of bytes)")

    bt = by_tool.get("Bash", [0, 0])[0]
    print(f"\nBash is {100*bt/total:.1f}% of tool bytes — the only part a "
          f"PreToolUse hook can reach.")
    if bt:
        print(f"\n{'command':<22}{'MB':>9}{'% of Bash':>11}{'calls':>8}{'avg B':>9}")
        for k, (n, c) in sorted(by_cmd.items(), key=lambda x: -x[1][0])[:a.top]:
            print(f"{k:<22}{n/1e6:>9.2f}{100*n/bt:>10.1f}%{c:>8}{n//max(c,1):>9}")


if __name__ == "__main__":
    main()
