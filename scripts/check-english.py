#!/usr/bin/env python3
"""Fail if Rust source contains non-ASCII (CJK) text in code (non-comment) lines.

The fastbrowser kernel is English-only for everything a host, the CLI, an LLM, or
the C ABI can see. Comments may still be non-English during migration, but code
and string literals must be ASCII.

Usage:
    python3 scripts/check-english.py [paths...]   # default: src
Exit code 1 if any violation is found.
"""

import re
import sys
import pathlib

# CJK ideographs + CJK punctuation + fullwidth forms.
CJK = re.compile(r"[\u3000-\u303f\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uff00-\uffef]")


def strip_line_comment(line: str) -> str:
    """Remove a trailing `//` comment, ignoring `//` inside double-quoted strings."""
    out = []
    in_str = False
    esc = False
    i = 0
    while i < len(line):
        c = line[i]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            out.append(c)
        else:
            if c == '"':
                in_str = True
                out.append(c)
            elif c == "/" and i + 1 < len(line) and line[i + 1] == "/":
                break
            else:
                out.append(c)
        i += 1
    return "".join(out)


def main(paths):
    roots = [pathlib.Path(p) for p in paths] or [pathlib.Path("src")]
    files = []
    for r in roots:
        if r.is_file():
            files.append(r)
        elif r.exists():
            files += sorted(r.rglob("*.rs"))

    bad = []
    for f in files:
        try:
            text = f.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for n, line in enumerate(text.splitlines(), 1):
            code = strip_line_comment(line)
            if CJK.search(code):
                bad.append((f, n, code.strip()))

    if bad:
        print("Non-English (CJK) text found in code (non-comment) lines:\n")
        for f, n, line in bad:
            print(f"  {f}:{n}: {line[:160]}")
        print(
            f"\n{len(bad)} violation(s). The kernel is English-only: "
            "keep code and user/LLM-visible strings in English."
        )
        return 1

    print(f"OK: checked {len(files)} Rust file(s); no CJK in code lines.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
