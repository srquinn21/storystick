"""Low-level STEP file tokenizing: treat the file as a graph of
`#id = ENTITY(args...)` records, addressable by id. No domain knowledge of
solids, assemblies, or parts lives here -- just the text-to-graph layer.
"""

from __future__ import annotations

import re

ENTITY_RE = re.compile(r"^#(\d+)\s*=\s*(.+);\s*$")
TYPE_RE = re.compile(r"^([A-Z_0-9]+)\((.*)\)$")


def split_args(s):
    """Split top-level comma-separated args, respecting nesting and quotes."""
    depth = 0
    in_str = False
    cur = ""
    args = []
    for ch in s:
        if ch == "'":
            in_str = not in_str
            cur += ch
        elif in_str:
            cur += ch
        elif ch == "(":
            depth += 1
            cur += ch
        elif ch == ")":
            depth -= 1
            cur += ch
        elif ch == "," and depth == 0:
            args.append(cur)
            cur = ""
        else:
            cur += ch
    if cur:
        args.append(cur)
    return args


def unquote(s):
    s = s.strip()
    if s.startswith("'") and s.endswith("'"):
        return s[1:-1]
    return s


def refs(s):
    return [int(x) for x in re.findall(r"#(\d+)", s)]


def parse_entities(path):
    entities = {}
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            m = ENTITY_RE.match(line.strip())
            if m:
                entities[int(m.group(1))] = m.group(2)
    return entities


def typed(entities, id_):
    raw = entities.get(id_)
    if raw is None:
        return None, None
    m = TYPE_RE.match(raw)
    if not m:
        return None, raw
    return m.group(1), split_args(m.group(2))
