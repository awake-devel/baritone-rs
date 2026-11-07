#!/usr/bin/env python3
# baritone-rs port ledger: one row per upstream (Baritone) method in scope.
# Written for this repository. It reads Baritone's Java source (cabaletta/baritone, LGPL-3.0) from a git
# checkout and never copies any of it into this repository.
#
# This file is part of baritone-rs.
#
# baritone-rs is free software: you can redistribute it and/or modify
# it under the terms of the GNU Lesser General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# baritone-rs is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU Lesser General Public License for more details.
#
# You should have received a copy of the GNU Lesser General Public License
# along with baritone-rs.  If not, see <https://www.gnu.org/licenses/>.
"""Ledger of upstream methods and how the port treats each one. See docs/PORTING-SAFETY.md.

Usage:
  ledger.py [--upstream DIR] [--ledger FILE] build [--rev REV]
  ledger.py [--upstream DIR] [--ledger FILE] check
  ledger.py [--upstream DIR] [--ledger FILE] upstream-diff REV

DIR is a git checkout of upstream, or the environment variable BARITONE_UPSTREAM.
Methods are read with `git show REV:path`, so the working tree state does not matter.

Parser limits (on purpose): it finds methods and constructors declared directly in a type body, in nested
types and in enum constant bodies. Code inside method bodies (lambdas, anonymous and local classes) belongs
to the enclosing method. Static initializer blocks and fields are not rows.

Row keys: class (nested types as Outer.Inner, enum constant bodies as Enum.CONSTANT), and method. The method
is the bare name, or name(Type,Type) when the class overloads that name. A constructor is <init>.
"""
import hashlib
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
DEFAULT_LEDGER = os.path.join(REPO, "docs", "port-ledger.tsv")
DEFAULT_REV = "1b8e87c"
RUST_ROOTS = ["mod/engine/src"]
COLUMNS = ["class", "method", "status", "commit", "hash", "covered_by", "note"]
STATUSES = ("port", "redesign", "drop", "later", "missing")

API = "src/api/java/baritone/api/"
MAIN = "src/main/java/baritone/"
# A path ending in "/" is a directory, searched recursively for .java files.
SCOPE = [
    API + "pathing/movement/ActionCosts.java",
    MAIN + "pathing/movement/Movement.java",
    MAIN + "pathing/movement/MovementHelper.java",
    MAIN + "pathing/movement/CalculationContext.java",
    MAIN + "pathing/movement/Moves.java",
    MAIN + "pathing/movement/movements/",
    MAIN + "pathing/precompute/",
    MAIN + "pathing/calc/",
    MAIN + "pathing/path/",
    API + "pathing/goals/",
    API + "pathing/calc/IPath.java",
    MAIN + "utils/pathing/PathBase.java",
    MAIN + "utils/pathing/Favoring.java",
    MAIN + "utils/pathing/Avoidance.java",
    MAIN + "behavior/PathingBehavior.java",
    MAIN + "behavior/LookBehavior.java",
    MAIN + "utils/BlockStateInterface.java",
    MAIN + "utils/ToolSet.java",
    MAIN + "utils/PathingControlManager.java",
    MAIN + "utils/pathing/BetterWorldBorder.java",
    MAIN + "utils/InputOverrideHandler.java",
    MAIN + "utils/PlayerMovementInput.java",
    MAIN + "pathing/movement/MovementState.java",
    MAIN + "pathing/movement/MovementOption.java",
    API + "utils/Rotation.java",
    API + "utils/RotationUtils.java",
    API + "utils/VecUtils.java",
    API + "command/datatypes/RelativeCoordinate.java",
    API + "command/datatypes/RelativeGoal.java",
    API + "command/datatypes/RelativeGoalBlock.java",
    API + "command/datatypes/RelativeGoalXZ.java",
    API + "command/datatypes/RelativeGoalYLevel.java",
    MAIN + "command/defaults/GotoCommand.java",
    MAIN + "process/CustomGoalProcess.java",
    MAIN + "process/MineProcess.java",
    MAIN + "process/GetToBlockProcess.java",
    MAIN + "process/FollowProcess.java",
    MAIN + "process/ExploreProcess.java",
    MAIN + "cache/CachedChunk.java",
    MAIN + "cache/CachedRegion.java",
    MAIN + "cache/CachedWorld.java",
    MAIN + "cache/ChunkPacker.java",
]


# ---------------------------------------------------------------- Java parsing

def lex(src):
    """Return (code, mask), both as long as src.

    code: comments blanked, strings and chars kept (used for hashing).
    mask: comments blanked, and the inside of strings and chars replaced by '_' (used for structure).
    Newlines are always kept.
    """
    n = len(src)
    code = list(src)
    mask = list(src)
    i = 0

    def blank(a, b, comment):
        for k in range(a, b):
            if src[k] != "\n":
                mask[k] = " " if comment else "_"
                if comment:
                    code[k] = " "

    while i < n:
        c = src[i]
        nx = src[i + 1] if i + 1 < n else ""
        if c == "/" and nx == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j, True)
            i = j
        elif c == "/" and nx == "*":
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            blank(i, j, True)
            i = j
        elif c == '"' and src.startswith('"""', i):
            j = i + 3
            while j < n and not src.startswith('"""', j):
                j += 2 if src[j] == "\\" else 1
            j = min(n, j)
            blank(i + 3, j, False)
            i = j + 3
        elif c == '"' or c == "'":
            j = i + 1
            while j < n and src[j] != c and src[j] != "\n":
                j += 2 if src[j] == "\\" else 1
            j = min(n, j)
            blank(i + 1, j, False)
            i = j + 1
        else:
            i += 1
    return "".join(code), "".join(mask)


PAIRS = {"{": "}", "(": ")", "[": "]"}


def match(mask, i):
    """Index of the bracket closing the one at i."""
    op = mask[i]
    cl = PAIRS[op]
    depth = 0
    for k in range(i, len(mask)):
        ch = mask[k]
        if ch == op:
            depth += 1
        elif ch == cl:
            depth -= 1
            if depth == 0:
                return k
    raise ValueError("unbalanced %r at offset %d" % (op, i))


def strip_annotations(h):
    """Blank out annotations (not @interface) in a header."""
    out = list(h)
    for m in re.finditer(r"@\s*(?!interface\b)[A-Za-z_][\w.]*", h):
        a = m.start()
        e = m.end()
        if out[a] == " ":
            continue  # inside an annotation argument list already blanked
        k = e
        while k < len(h) and h[k].isspace():
            k += 1
        if k < len(h) and h[k] == "(":
            e = match(h, k) + 1
        for x in range(a, e):
            out[x] = " "
    return "".join(out)


def split_top(s, sep=","):
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == sep and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    parts.append("".join(cur))
    return [p for p in parts if p.strip()]


def param_type(p):
    p = strip_annotations(p)
    while True:
        q = re.sub(r"<[^<>]*>", "", p)
        if q == p:
            break
        p = q
    p = re.sub(r"\s*\.\.\.\s*", "... ", p)
    p = re.sub(r"\bfinal\b", " ", p)
    toks = p.split()
    if len(toks) >= 2:
        toks = toks[:-1]
    t = "".join(toks)
    return re.sub(r"\b[a-z]\w*\.", "", t)


class Method:
    def __init__(self, cls, name, ptypes, start, end, text, mask_text):
        self.cls = cls
        self.name = name
        self.ptypes = ptypes
        self.start = start
        self.end = end
        self.text = text
        self.mask_text = mask_text
        self.key = name  # becomes a signature in parse_java if the name is overloaded
        self.file = ""

    def norm(self):
        """Text with whitespace collapsed outside string and char literals, where it is kept."""
        out, space = [], False
        for ch, mk in zip(self.text, self.mask_text):
            if ch.isspace() and mk != "_":
                space = True
                continue
            if space and out:
                out.append(" ")
            space = False
            out.append(ch)
        return "".join(out)

    def digest(self):
        return hashlib.sha256(self.norm().encode()).hexdigest()[:16]


TYPE_RE = re.compile(r"(?<![\w])(@\s*)?(class|interface|enum|record)\s+([A-Za-z_]\w*)")


def _skip_ws(mask, i, hi):
    while i < hi and (mask[i].isspace() or mask[i] == ";"):
        i += 1
    return i


def _scan_enum_constants(mask, code, i, hi, cls, out):
    """Parse the constant list of an enum body. Returns the position after the ';' (or hi)."""
    ident = re.compile(r"[A-Za-z_]\w*")
    while True:
        while i < hi and (mask[i].isspace() or mask[i] == ","):
            i += 1
        if i >= hi:
            return hi
        if mask[i] == ";":
            return i + 1
        if mask[i] == "@":
            m = re.compile(r"@\s*[A-Za-z_][\w.]*\s*").match(mask, i)
            i = m.end()
            if i < hi and mask[i] == "(":
                i = match(mask, i) + 1
            continue
        m = ident.match(mask, i)
        if not m:
            raise ValueError("cannot read enum constant at offset %d" % i)
        name = m.group()
        i = m.end()
        while i < hi and mask[i].isspace():
            i += 1
        if i < hi and mask[i] == "(":
            i = match(mask, i) + 1
            while i < hi and mask[i].isspace():
                i += 1
        if i < hi and mask[i] == "{":
            e = match(mask, i)
            _scan_members(mask, code, i + 1, e, cls + "." + name, False, name, out)
            i = e + 1


def _scan_members(mask, code, lo, hi, cls, is_enum, simple, out):
    i = lo
    if is_enum:
        i = _scan_enum_constants(mask, code, i, hi, cls, out)
    while i < hi:
        i = _skip_ws(mask, i, hi)
        if i >= hi:
            break
        m = i
        j = i
        while j < hi:
            c = mask[j]
            if c == "(":
                j = match(mask, j) + 1
                continue
            if c in ";{=":
                break
            j += 1
        if j >= hi:
            break
        c = mask[j]
        header = strip_annotations(mask[m:j])
        paren = header.find("(")
        before = header if paren < 0 else header[:paren]
        if c == "=":  # a field with an initializer: skip to its ';'
            k = j
            while k < hi:
                ch = mask[k]
                if ch in "({[":
                    k = match(mask, k) + 1
                elif ch == ";":
                    break
                else:
                    k += 1
            i = k + 1
            continue
        tm = TYPE_RE.search(before)
        if c == ";":
            if paren >= 0 and not tm:
                _add_method(code, mask, m, j + 1, header, paren, cls, simple, out)
            i = j + 1
            continue
        end = match(mask, j)  # c == "{"
        if tm:
            kind, name = tm.group(2), tm.group(3)
            sub = cls + "." + name if cls else name
            _scan_members(mask, code, j + 1, end, sub, kind == "enum" and not tm.group(1), name, out)
        elif paren >= 0:
            _add_method(code, mask, m, end + 1, header, paren, cls, simple, out)
        # otherwise an initializer block: skipped
        i = end + 1


def _add_method(code, mask, start, end, header, paren, cls, simple, out):
    nm = re.search(r"([A-Za-z_]\w*)\s*$", header[:paren])
    if not nm:
        raise ValueError("cannot read a method name in %r" % header[:paren].strip())
    name = nm.group(1)
    close = match(header, paren)
    ptypes = [param_type(p) for p in split_top(header[paren + 1:close])]
    if name == simple:
        lead = header[:nm.start()]
        while True:  # generics such as <T> before a constructor name
            q = re.sub(r"<[^<>]*>", " ", lead)
            if q == lead:
                break
            lead = q
        lead = re.sub(r"\b(public|protected|private|static|final|abstract|synchronized|native|strictfp)\b", " ", lead)
        if not lead.strip():
            name = "<init>"
    out.append(Method(cls, name, ptypes, start, end, code[start:end], mask[start:end]))


def parse_java(src):
    """List of Method for one source file. Overloaded names get a signature key."""
    code, mask = lex(src)
    out = []
    _scan_members(mask, code, 0, len(mask), "", False, "", out)
    counts = {}
    for m in out:
        counts[(m.cls, m.name)] = counts.get((m.cls, m.name), 0) + 1
    for m in out:
        if counts[(m.cls, m.name)] > 1:
            m.key = "%s(%s)" % (m.name, ",".join(m.ptypes))
    return out


# ---------------------------------------------------------------- git access

def git(up, *args):
    r = subprocess.run(["git", "-C", up, *args], capture_output=True, text=True, errors="replace")
    if r.returncode != 0:
        raise RuntimeError("git %s failed: %s" % (" ".join(args), r.stderr.strip()))
    return r.stdout


def short_rev(up, rev):
    return git(up, "rev-parse", "--short=7", rev + "^{commit}").strip()


def scope_files(up, rev):
    names = git(up, "ls-tree", "-r", "--name-only", rev).splitlines()
    have = set(names)
    files, missing = [], []
    for s in SCOPE:
        if s.endswith("/"):
            found = sorted(n for n in names if n.startswith(s) and n.endswith(".java"))
            if not found:
                missing.append(s)
            files += found
        elif s in have:
            files.append(s)
        else:
            missing.append(s)
    return files, missing


_cache = {}


def load_methods(up, rev):
    """Parse every in-scope file at rev. Returns ({(class, key): Method}, problems)."""
    ck = (os.path.abspath(up), rev)
    if ck in _cache:
        return _cache[ck]
    files, missing = scope_files(up, rev)
    problems = ["scope path not found: %s" % s for s in missing]
    table = {}
    for f in files:
        src = git(up, "show", "%s:%s" % (rev, f))
        try:
            ms = parse_java(src)
        except ValueError as e:
            problems.append("cannot parse %s: %s" % (f, e))
            continue
        for m in ms:
            k = (m.cls, m.key)
            if k in table:
                problems.append("duplicate method %s.%s in %s" % (k[0], k[1], f))
            m.file = f
            table[k] = m
    _cache[ck] = (table, problems)
    return table, problems


# ---------------------------------------------------------------- ledger file

def read_ledger(path):
    rows = []
    if not os.path.exists(path):
        return rows
    with open(path, encoding="utf-8") as fh:
        lines = fh.read().split("\n")
    if lines[0].split("\t") != COLUMNS:
        raise ValueError("%s: header must be: %s" % (path, "\t".join(COLUMNS)))
    for n, line in enumerate(lines[1:], 2):
        if not line.strip():
            continue
        cells = line.split("\t")
        if len(cells) != len(COLUMNS):
            raise ValueError("%s:%d: expected %d columns, found %d" % (path, n, len(COLUMNS), len(cells)))
        row = dict(zip(COLUMNS, cells))
        row["line"] = n
        rows.append(row)
    return rows


def write_ledger(path, rows):
    rows = sorted(rows, key=lambda r: (r["class"], r["method"]))
    d = os.path.dirname(os.path.abspath(path))
    os.makedirs(d, exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\t".join(COLUMNS) + "\n")
        for r in rows:
            fh.write("\t".join(re.sub(r"[\t\r\n]+", " ", r[c]) for c in COLUMNS) + "\n")


# ---------------------------------------------------------------- Rust citations

CITE = re.compile(r"^\s*///\s*`([A-Za-z_][\w.]*)(\(([^`]*)\))?`\s*:")


def rust_citations(repo):
    """List of dicts: file, line, class, method (as written), is_fn."""
    found = []
    for root in RUST_ROOTS:
        for dp, _, fs in os.walk(os.path.join(repo, root)):
            for f in sorted(fs):
                if not f.endswith(".rs"):
                    continue
                path = os.path.join(dp, f)
                with open(path, encoding="utf-8") as fh:
                    lines = fh.read().split("\n")
                i = 0
                while i < len(lines):
                    if not lines[i].lstrip().startswith("///"):
                        i += 1
                        continue
                    j = i
                    cites = []
                    while j < len(lines) and lines[j].lstrip().startswith("///"):
                        m = CITE.match(lines[j])
                        if m and "." in m.group(1):
                            cites.append((j + 1, m))
                        j += 1
                    k = j
                    while k < len(lines) and (not lines[k].strip() or lines[k].lstrip().startswith("#[")):
                        k += 1
                    is_fn = k < len(lines) and re.search(r"\bfn\s+\w+", lines[k]) is not None
                    for ln, m in cites:
                        cls, meth = m.group(1).rsplit(".", 1)
                        if m.group(2):
                            meth += "(" + re.sub(r"\s+", "", m.group(3)) + ")"
                        found.append({"file": os.path.relpath(path, repo), "line": ln, "class": cls,
                                      "method": meth, "is_fn": is_fn})
                    i = j
    return found


# ---------------------------------------------------------------- commands

def need_upstream(args):
    up = args["upstream"] or os.environ.get("BARITONE_UPSTREAM")
    if not up:
        raise SystemExit("error: give --upstream DIR or set BARITONE_UPSTREAM (a git checkout of upstream)")
    return up


def cmd_build(up, ledger, rev):
    rev = short_rev(up, rev)
    table, problems = load_methods(up, rev)
    rows = read_ledger(ledger)
    have = {(r["class"], r["method"]) for r in rows}
    added = 0
    for (cls, key), m in table.items():
        if (cls, key) not in have:
            rows.append({"class": cls, "method": key, "status": "missing", "commit": rev,
                         "hash": m.digest(), "covered_by": "", "note": ""})
            added += 1
    gone = [r for r in rows if (r["class"], r["method"]) not in table]
    write_ledger(ledger, rows)
    print("build at %s: %d methods in scope, %d rows added, %d rows total" % (rev, len(table), added, len(rows)))
    for r in gone:
        print("NOT FOUND at %s (row kept): %s.%s" % (rev, r["class"], r["method"]))
    for p in problems:
        print("PROBLEM: " + p)
    return 1 if problems else 0


def resolve_cite(by_name, cls, meth):
    """(row, None) for a citation, or (None, reason)."""
    row = by_name.get((cls, meth))
    if row:
        return row, None
    if "(" not in meth:
        over = [r["method"] for (c, k), r in by_name.items() if c == cls and k.startswith(meth + "(")]
        if over:
            return None, "ambiguous, the name is overloaded: cite one of " + ", ".join(sorted(over))
    return None, "no row"


def cmd_check(up, ledger, repo):
    errors = []
    rows = read_ledger(ledger)
    by_name = {}
    for r in rows:
        key = (r["class"], r["method"])
        where = "%s:%d" % (os.path.basename(ledger), r["line"])
        if key in by_name:
            errors.append("%s: duplicate row %s.%s" % (where, *key))
        by_name[key] = r
        if r["status"] not in STATUSES:
            errors.append("%s: %s.%s: unknown status %r" % (where, *key, r["status"]))
        if not r["commit"] or not r["hash"]:
            errors.append("%s: %s.%s: commit and hash are required" % (where, *key))
    by_commit = {}
    for r in rows:
        by_commit.setdefault(r["commit"], []).append(r)
    for commit, rs in sorted(by_commit.items()):
        try:
            table, problems = load_methods(up, commit)
        except RuntimeError as e:
            errors.append("cannot read upstream at %s: %s" % (commit, e))
            continue
        errors += ["at %s: %s" % (commit, p) for p in problems]
        for r in rs:
            m = table.get((r["class"], r["method"]))
            if m is None:
                errors.append("%s.%s: method no longer exists at its recorded commit %s" %
                              (r["class"], r["method"], commit))
            elif m.digest() != r["hash"]:
                errors.append("%s.%s: hash mismatch at %s (row %s, text %s)" %
                              (r["class"], r["method"], commit, r["hash"], m.digest()))
    cites = rust_citations(repo)
    cited = set()
    for c in cites:
        where = "%s:%d" % (c["file"], c["line"])
        row, why = resolve_cite(by_name, c["class"], c["method"])
        if row is None:
            errors.append("%s: cites %s.%s: %s" % (where, c["class"], c["method"], why))
            continue
        if row["status"] not in ("port", "redesign"):
            errors.append("%s: cites %s.%s but its row has status %s" %
                          (where, row["class"], row["method"], row["status"]))
        if c["is_fn"]:
            cited.add((row["class"], row["method"]))
        else:
            errors.append("%s: the citation of %s.%s is not on a function" %
                          (where, row["class"], row["method"]))
    for r in rows:
        if r["status"] == "port" and (r["class"], r["method"]) not in cited:
            errors.append("%s.%s: status port, but no Rust function cites it" % (r["class"], r["method"]))
    counts = {s: sum(1 for r in rows if r["status"] == s) for s in STATUSES}
    print("checked %d rows and %d Rust citations (%s)" %
          (len(rows), len(cites), ", ".join("%s %d" % (s, counts[s]) for s in STATUSES)))
    for e in errors:
        print("FAIL: " + e)
    print("check FAILED, %d problems" % len(errors) if errors else "check passed")
    return 1 if errors else 0


def cmd_diff(up, ledger, new_rev):
    new_rev = short_rev(up, new_rev)
    rows = read_ledger(ledger)
    new_table, problems = load_methods(up, new_rev)
    changed = removed = 0
    for r in rows:
        m = new_table.get((r["class"], r["method"]))
        if m is None:
            print("REMOVED  %s.%s [%s] (recorded at %s)" % (r["class"], r["method"], r["status"], r["commit"]))
            removed += 1
        elif m.digest() != r["hash"]:
            print("CHANGED  %s.%s [%s] (recorded at %s)" % (r["class"], r["method"], r["status"], r["commit"]))
            changed += 1
    known = {(r["class"], r["method"]) for r in rows}
    added = sorted(k for k in new_table if k not in known)
    for cls, key in added:
        print("NEW      %s.%s (no row yet)" % (cls, key))
    for p in problems:
        print("PROBLEM: " + p)
    print("against %s: %d changed, %d removed, %d new" % (new_rev, changed, removed, len(added)))
    return 0


def main(argv):
    args = {"upstream": None, "ledger": DEFAULT_LEDGER, "repo": REPO}
    rest = []
    it = iter(argv)
    for a in it:
        if a in ("--upstream", "--ledger", "--repo"):
            try:
                args[a[2:]] = next(it)
            except StopIteration:
                print("error: %s needs a value" % a)
                return 2
        else:
            rest.append(a)
    if not rest or rest[0] in ("-h", "--help"):
        print(__doc__)
        return 2
    cmd = rest[0]
    if cmd == "build" and (len(rest) == 1 or (len(rest) == 3 and rest[1] == "--rev")):
        return cmd_build(need_upstream(args), args["ledger"], rest[2] if len(rest) == 3 else DEFAULT_REV)
    if cmd == "check" and len(rest) == 1:
        return cmd_check(need_upstream(args), args["ledger"], args["repo"])
    if cmd == "upstream-diff" and len(rest) == 2:
        return cmd_diff(need_upstream(args), args["ledger"], rest[1])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
