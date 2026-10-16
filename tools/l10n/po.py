"""Read and write the gettext PO files in ``lang/``.

The same subset the game's reader (``ff_core::l10n::po``) understands:
single ``msgid``/``msgstr`` pairs, multi-line strings, C escapes, ``#.``
notes from the tools, ``#`` notes from translators, ``#:`` references,
``#,`` flags and ``#~`` obsolete entries.
"""

from __future__ import annotations

from dataclasses import dataclass, field

WRAP = 76


@dataclass
class Entry:
    msgid: str
    msgstr: str = ""
    # gettext's ``msgctxt``: tells apart two entries with the same English.
    msgctxt: str | None = None
    refs: list[str] = field(default_factory=list)
    comments: list[str] = field(default_factory=list)
    translator_comments: list[str] = field(default_factory=list)
    flags: list[str] = field(default_factory=list)
    obsolete: bool = False
    line: int = 0

    @property
    def key(self) -> tuple[str | None, str]:
        return (self.msgctxt, self.msgid)

    @property
    def fuzzy(self) -> bool:
        return "fuzzy" in self.flags


class PoError(ValueError):
    pass


def _unquote(token: str, line: int) -> str:
    token = token.strip()
    if len(token) < 2 or not (token.startswith('"') and token.endswith('"')):
        raise PoError(f"line {line}: expected a quoted string, found {token!r}")
    out = []
    body = token[1:-1]
    i = 0
    while i < len(body):
        c = body[i]
        if c == "\\":
            if i + 1 >= len(body):
                raise PoError(f"line {line}: string ends in a backslash")
            nxt = body[i + 1]
            mapped = {"n": "\n", "t": "\t", "r": "\r", '"': '"', "\\": "\\"}.get(nxt)
            if mapped is None:
                raise PoError(f"line {line}: unknown escape \\{nxt}")
            out.append(mapped)
            i += 2
            continue
        if c == '"':
            raise PoError(f"line {line}: unescaped quote inside a string")
        out.append(c)
        i += 1
    return "".join(out)


def quote(text: str) -> str:
    return (
        '"'
        + text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t")
        + '"'
    )


def parse_po(text: str) -> list[Entry]:
    entries: list[Entry] = []
    cur: Entry | None = None
    pending = Entry(msgid="")
    field_name = None

    def finish() -> None:
        nonlocal cur, pending
        if cur is not None:
            entries.append(cur)
        cur = None
        pending = Entry(msgid="")

    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        obsolete = False
        if line.startswith("#~"):
            obsolete = True
            line = line[2:].strip()
            if not line:
                continue
        if not line:
            if cur is not None and field_name == "msgstr":
                finish()
            continue
        if line.startswith("#") and not obsolete:
            if cur is not None and field_name == "msgstr":
                finish()
            if line.startswith("#."):
                pending.comments.append(line[2:].strip())
            elif line.startswith("#:"):
                pending.refs.extend(line[2:].split())
            elif line.startswith("#,"):
                pending.flags.extend(f.strip() for f in line[2:].split(",") if f.strip())
            elif line.startswith("#|"):
                pass
            else:
                pending.translator_comments.append(line[1:].strip())
            continue
        if line.startswith("msgctxt "):
            if cur is not None:
                finish()
            pending.msgctxt = _unquote(line[8:], number)
            field_name = "msgctxt"
            continue
        if line.startswith("msgid "):
            if cur is not None:
                finish()
            cur = pending
            cur.msgid = _unquote(line[6:], number)
            cur.line = number
            cur.obsolete = obsolete
            field_name = "msgid"
            continue
        if line.startswith("msgstr "):
            if cur is None:
                raise PoError(f"line {number}: msgstr without a msgid")
            cur.msgstr = _unquote(line[7:], number)
            field_name = "msgstr"
            continue
        if line.startswith('"'):
            if field_name == "msgctxt":
                pending.msgctxt = (pending.msgctxt or "") + _unquote(line, number)
                continue
            if cur is None or field_name is None:
                raise PoError(f"line {number}: a string continues nothing")
            value = _unquote(line, number)
            if field_name == "msgid":
                cur.msgid += value
            else:
                cur.msgstr += value
            continue
        raise PoError(f"line {number}: cannot read {line!r}")
    finish()
    return entries


def _string_lines(keyword: str, text: str, prefix: str = "") -> list[str]:
    """``msgid "..."``, wrapped the way gettext tools wrap long strings."""
    if len(text) + len(keyword) + 3 <= WRAP and "\n" not in text[:-1]:
        return [f"{prefix}{keyword} {quote(text)}"]
    chunks: list[str] = []
    for part in text.splitlines(keepends=True):
        while len(part) > WRAP:
            cut = part.rfind(" ", 0, WRAP)
            cut = WRAP if cut <= 0 else cut + 1
            chunks.append(part[:cut])
            part = part[cut:]
        if part:
            chunks.append(part)
    return [f'{prefix}{keyword} ""'] + [f"{prefix}{quote(c)}" for c in chunks]


def write_po(entries: list[Entry], refs: bool) -> str:
    blocks = []
    for entry in entries:
        lines: list[str] = []
        for note in entry.translator_comments:
            lines.append(f"# {note}".rstrip())
        for note in entry.comments:
            lines.append(f"#. {note}")
        if refs and entry.refs:
            lines.append("#: " + " ".join(entry.refs))
        if entry.flags:
            lines.append("#, " + ", ".join(entry.flags))
        prefix = "#~ " if entry.obsolete else ""
        if entry.msgctxt is not None:
            lines += _string_lines("msgctxt", entry.msgctxt, prefix)
        lines += _string_lines("msgid", entry.msgid, prefix)
        lines += _string_lines("msgstr", entry.msgstr, prefix)
        blocks.append("\n".join(lines))
    return "\n\n".join(blocks) + "\n"
