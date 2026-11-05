#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Checks that the repository holds nothing that must not be public.

Checks every file git lists as tracked, or as untracked and not ignored (git ls-files --cached --others
--exclude-standard), so a new file is checked before it is added. Ignored files are not checked. With --history every blob and commit message ever recorded.
Fails on: .jar/.class files, golden or generated data files, files over a size limit, Java sources outside
tools/, absolute home paths and similar private strings, email addresses, and anything on a local deny list.

The deny list is a plain text file (one string per line, # for comments) whose path comes from the
environment variable BARITONE_RS_DENYLIST. The file stays untracked, so the private strings never get committed.

Exit status: 0 clean, 1 findings, 3 clean but a part was skipped (no deny list configured).
"""
import argparse
import os
import re
import subprocess
import sys

MAX_BYTES = 200_000
SOURCE_EXT = {".rs", ".py", ".java", ".md", ".toml", ".txt", ".lock", ".json", ".tsv", ".yml", ".yaml"}
BAD_EXT = {".jar", ".class", ".war", ".zip", ".nbt", ".mca", ".dat"}
# Built from pieces so that this file does not contain the strings it looks for.
PRIVATE = [
    ("/" + "home/", "absolute home path"),
    ("/" + "Users/", "absolute home path"),
    ("/" + "tmp/claude", "scratch path"),
    ("/" + "run/user/", "runtime path"),
]
# Bounded, so that a long run of letters cannot make the scan quadratic.
EMAIL = re.compile(r"[A-Za-z0-9._%+-]{1,64}@[A-Za-z0-9-]{1,63}(?:\.[A-Za-z0-9-]{1,63}){1,8}")
EMAIL_OK = ("noreply.github.com", "example.com", "example.org", "noreply@anthropic.com")  # no-reply addresses only
GOLDEN_NAME = re.compile(r"golden|generated|\.dump$", re.I)
ENV_DENY = "BARITONE_RS_DENYLIST"


def git(repo, *args, raw=False):
    out = subprocess.run(["git", "-C", repo, *args], capture_output=True, check=True).stdout
    return out if raw else out.decode("utf-8", "replace")


def load_denylist(path):
    """Returns the lowercase deny strings, or None if no list is configured."""
    if not path:
        return None
    with open(path, encoding="utf-8") as f:
        return [ln.strip().lower() for ln in f if ln.strip() and not ln.lstrip().startswith("#")]


def check_name_and_size(path, size):
    """Checks one file by name and size. Returns a list of messages."""
    out = []
    base = os.path.basename(path)
    ext = os.path.splitext(base)[1].lower()
    is_license = base.upper().startswith("LICENSE")
    if ext in BAD_EXT:
        out.append(f"{path}: forbidden file type {ext}")
    if GOLDEN_NAME.search(base) and ext not in SOURCE_EXT:
        out.append(f"{path}: looks like golden or generated data (by name)")
    if ext == ".java" and not path.startswith("tools/"):
        out.append(f"{path}: Java source outside tools/")
    if size > MAX_BYTES and not is_license:
        out.append(f"{path}: {size} bytes is over the {MAX_BYTES} byte limit")
    return out


def check_text(label, data, deny):
    """Checks file content or a commit message. Returns a list of messages."""
    if b"\0" in data:
        return []
    text = data.decode("utf-8", "replace")
    low = text.lower()
    out = []
    for needle, what in PRIVATE:
        i = text.find(needle)
        if i >= 0:
            out.append(f"{label}: {what} (line {text.count(chr(10), 0, i) + 1})")
    for m in EMAIL.finditer(text):
        if not m.group(0).lower().endswith(EMAIL_OK):
            out.append(f"{label}: email address (line {text.count(chr(10), 0, m.start()) + 1})")
            break
    for d in deny or []:
        i = low.find(d)
        if i >= 0:
            out.append(f"{label}: matches deny list entry #{deny.index(d) + 1} (line {text.count(chr(10), 0, i) + 1})")
    return out


def check_tree(repo, deny):
    out = []
    files = [f for f in git(repo, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0") if f]
    for f in files:
        full = os.path.join(repo, f)
        if os.path.islink(full) or not os.path.exists(full):
            continue
        out += check_name_and_size(f, os.path.getsize(full))
        for d in deny or []:
            if d in f.lower():
                out.append(f"{f}: file name matches the deny list")
        if os.path.getsize(full) <= MAX_BYTES * 5:
            with open(full, "rb") as fh:
                out += check_text(f, fh.read(), deny)
    return out, len(files)


def check_history(repo, deny):
    out = []
    objs = git(repo, "rev-list", "--objects", "--all").splitlines()
    named = {}
    for ln in objs:
        sha, _, path = ln.partition(" ")
        if path:
            named.setdefault(sha, path)
    if named:
        inp = "\n".join(named).encode() + b"\n"
        res = subprocess.run(["git", "-C", repo, "cat-file", "--batch-check"], input=inp, capture_output=True, check=True)
        for ln in res.stdout.decode().splitlines():
            sha, kind, size = ln.split()
            if kind != "blob":
                continue
            path, size = named[sha], int(size)
            msgs = check_name_and_size(path, size)
            if size <= MAX_BYTES * 5:
                msgs += check_text(path, git(repo, "cat-file", "blob", sha, raw=True), deny)
            out += [f"history {sha[:10]}: {m}" for m in msgs]
    for commit in git(repo, "rev-list", "--all").split():
        meta = git(repo, "show", "-s", "--format=%an%n%ae%n%cn%n%ce%n%B", commit, raw=True)
        out += [f"history commit {commit[:10]}: {m}" for m in check_text("message or identity", meta, deny)]
    return out, len(named)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
    ap.add_argument("--history", action="store_true", help="also check every blob and commit message in history")
    a = ap.parse_args(argv)
    repo = os.path.abspath(a.repo)
    deny = load_denylist(os.environ.get(ENV_DENY))
    findings, n = check_tree(repo, deny)
    print(f"checked {n} files (tracked, and untracked but not ignored)")
    if a.history:
        more, m = check_history(repo, deny)
        findings += more
        print(f"checked {m} named objects in history")
    if findings:
        print(f"FAIL: {len(findings)} finding(s)")
        for f in findings:
            print("  " + f)
        return 1
    if deny is None:
        print(f"PASS, but SKIPPED the deny list: {ENV_DENY} is not set")
        return 3
    print(f"PASS (deny list: {len(deny)} entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
