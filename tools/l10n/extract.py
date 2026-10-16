"""Rebuild the translation template and bring every language file up to date.

    uv run python tools/l10n/extract.py           # rewrite lang/*.pot and *.po
    uv run python tools/l10n/extract.py --check   # fail if they are stale

The template, ``lang/freight-fate.pot``, lists every line the game can
speak: the player-facing literals in ``crates/`` (see ``rust_strings``),
the spoken fields of ``data/buffs.json``, and ``lang/extra.pot``, which
holds lines the source never spells out whole (assembled from pieces at run
time) that a maintainer added by hand.

Each ``lang/<code>.po`` keeps every translation it has. A line new to the
template arrives untranslated; a line the game no longer says is dropped,
unless it was translated, in which case it is kept as an obsolete ``#~``
entry so the work is not lost if the line comes back reworded.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from po import Entry, parse_po, write_po  # noqa: E402
from rust_strings import extract  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
LANG = ROOT / "lang"
TEMPLATE = LANG / "freight-fate.pot"
EXTRA = LANG / "extra.pot"

# Every language with a catalog, matching ff_core::l10n::LANGUAGES.
LANGUAGES = {
    "es": ("Spanish", "nplurals=2; plural=(n != 1);"),
    "fr": ("French", "nplurals=2; plural=(n > 1);"),
    "de": ("German", "nplurals=2; plural=(n != 1);"),
    "pt_BR": ("Brazilian Portuguese", "nplurals=2; plural=(n != 1);"),
    "it": ("Italian", "nplurals=2; plural=(n != 1);"),
    "ru": (
        "Russian",
        "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && "
        "(n%100<10 || n%100>=20) ? 1 : 2);",
    ),
    "pl": (
        "Polish",
        "nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
    ),
    "zh_CN": ("Simplified Chinese", "nplurals=1; plural=0;"),
    "ja": ("Japanese", "nplurals=1; plural=0;"),
}

CRATES = ["ff-core", "freight-fate"]
SENTENCE_BREAK = re.compile(r"(?<=[.!?])\s+(?=[A-Z0-9{\"])")
PLACEHOLDER = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}")
# A placeholder written straight after a word: another piece of the line.
GLUED_TAIL = re.compile(r"(?<=[A-Za-z.])\{[A-Za-z_][A-Za-z0-9_]*\}$")
BUFF_FIELDS = ("label", "help", "purchased", "worn_off")

# Lines that one file says with a meaning of their own: (file, English) to
# the msgctxt that tells them apart. The game never looks up an entry with
# a context (it matches finished lines), so the plain entry is the one
# spoken; the context entry exists so a translator sees the two uses apart.
CONTEXTS = {
    # The controller's Back button, named as the controller prints it; the
    # plain "Back" is the menu row that leaves a screen.
    ("crates/freight-fate/src/bindings/names.rs", "Back"): "controller button",
}


def template_entries() -> list[Entry]:
    found = extract(ROOT, CRATES)
    entries: dict[str, Entry] = {}
    contextual: list[Entry] = []
    for template in found.values():
        comments = []
        if template.placeholders:
            comments.append(
                "placeholders: " + ", ".join("{" + p + "}" for p in template.placeholders)
            )
        refs = list(template.refs)
        for ref in list(refs):
            ctxt = CONTEXTS.get((ref, template.msgid))
            if ctxt is not None:
                refs.remove(ref)
                contextual.append(
                    Entry(msgid=template.msgid, msgctxt=ctxt, refs=[ref], line=template.line)
                )
        if not refs:
            continue
        entries[template.msgid] = Entry(
            msgid=template.msgid,
            refs=refs[:4],
            comments=comments,
            line=template.line,
        )
    # A long line built in one format! call often reaches the player with
    # one of its optional pieces changed (or empty), so each sentence of a
    # template with placeholders is a template too: the game translates sentence by sentence when the whole
    # line does not match.
    for template in list(entries.values()):
        sentences = SENTENCE_BREAK.split(template.msgid)
        if len(sentences) < 2 or not PLACEHOLDER.search(template.msgid):
            continue
        for sentence in sentences:
            sentence = sentence.strip()
            literal = PLACEHOLDER.sub("", sentence)
            if sentence in entries or sum(c.isalpha() for c in literal) < 3:
                continue
            names = [m.group(0)[1:-1] for m in PLACEHOLDER.finditer(sentence)]
            entries[sentence] = Entry(
                msgid=sentence,
                refs=template.refs[:1],
                line=template.line,
                comments=(
                    ["placeholders: " + ", ".join("{" + p + "}" for p in names)] if names else []
                )
                + ["one sentence of a longer line"],
            )
    # A piece the game appends to a longer line (", automatic speed control
    # paused{keeper_target}") reaches the player inside that line, where
    # the game translates it clause by clause. Each of its clauses is a
    # template, without the joining comma or a piece glued on after it.
    for template in list(entries.values()):
        if not template.msgid.startswith(", "):
            continue
        body = GLUED_TAIL.sub("", template.msgid[2:].strip())
        for clause in body.split(", "):
            clause = clause.strip()
            literal = PLACEHOLDER.sub("", clause)
            if clause in entries or sum(c.isalpha() for c in literal) < 3:
                continue
            names = [m.group(0)[1:-1] for m in PLACEHOLDER.finditer(clause)]
            entries[clause] = Entry(
                msgid=clause,
                refs=template.refs[:1],
                line=template.line,
                comments=(
                    ["placeholders: " + ", ".join("{" + p + "}" for p in names)] if names else []
                )
                + ["one clause of a longer line"],
            )
    buffs = json.loads((ROOT / "data" / "buffs.json").read_text(encoding="utf-8"))
    for buff in buffs.values():
        for field in BUFF_FIELDS:
            text = buff.get(field)
            if isinstance(text, str) and text.strip():
                ref = "data/buffs.json"
                if text in entries:
                    if ref not in entries[text].refs:
                        entries[text].refs.append(ref)
                else:
                    entries[text] = Entry(msgid=text, refs=[ref], line=len(entries))
    if EXTRA.exists():
        for extra in parse_po(EXTRA.read_text(encoding="utf-8")):
            if not extra.msgid:
                continue
            if extra.msgid in entries:
                entries[extra.msgid].comments.extend(extra.comments)
            else:
                entries[extra.msgid] = Entry(
                    msgid=extra.msgid,
                    refs=extra.refs or ["lang/extra.pot"],
                    comments=extra.comments,
                    line=extra.line,
                )
    return sorted(list(entries.values()) + contextual, key=_order)


def _order(entry: Entry) -> tuple[str, int, str]:
    """Group lines by the file that says them, in the order it says them."""
    return (entry.refs[0] if entry.refs else "", entry.line, entry.msgid)


TEMPLATE_HEADER = (
    "Project-Id-Version: Freight Fate\n"
    "Report-Msgid-Bugs-To: https://github.com/orinks-games/freight-fate/issues\n"
    "MIME-Version: 1.0\n"
    "Content-Type: text/plain; charset=UTF-8\n"
    "Content-Transfer-Encoding: 8bit\n"
)


def language_codes() -> list[str]:
    """The built-in languages, then any other ``lang/*.po`` a translator added."""
    added = sorted(p.stem for p in LANG.glob("*.po") if p.stem not in LANGUAGES)
    return list(LANGUAGES) + added


def language_header(code: str, existing: list[Entry]) -> str:
    """A built-in language's header, or the one an added file brought."""
    if code not in LANGUAGES:
        brought = next((e.msgstr for e in existing if not e.msgid), "")
        return brought or TEMPLATE_HEADER + f"Language: {code}\n"
    name, plural = LANGUAGES[code]
    return TEMPLATE_HEADER + f"Language: {code}\nPlural-Forms: {plural}\nX-Language-Name: {name}\n"


def merge(code: str, template: list[Entry], existing: list[Entry]) -> list[Entry]:
    """The language file for ``template``, keeping ``existing`` translations."""
    known = {e.key: e for e in existing if e.msgid and not e.obsolete}
    obsolete = {e.key: e for e in existing if e.msgid and e.obsolete}
    out = [Entry(msgid="", msgstr=language_header(code, existing))]
    for entry in template:
        old = known.pop(entry.key, None) or obsolete.pop(entry.key, None)
        merged = Entry(
            msgid=entry.msgid,
            msgctxt=entry.msgctxt,
            msgstr=old.msgstr if old else "",
            # The template's notes come first; a translator's own `#` notes
            # on the entry survive the merge.
            comments=entry.comments,
            translator_comments=old.translator_comments if old else [],
            flags=old.flags if old else [],
        )
        out.append(merged)
    for gone in list(known.values()) + list(obsolete.values()):
        if gone.msgstr:
            gone.obsolete = True
            gone.refs = []
            gone.comments = []
            out.append(gone)
    return out


def render_all() -> dict[Path, str]:
    template = template_entries()
    files = {TEMPLATE: write_po([Entry(msgid="", msgstr=TEMPLATE_HEADER)] + template, refs=True)}
    for code in language_codes():
        path = LANG / f"{code}.po"
        existing = parse_po(path.read_text(encoding="utf-8")) if path.exists() else []
        files[path] = write_po(merge(code, template, existing), refs=False)
    return files


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="fail if any file is stale")
    args = parser.parse_args(argv)
    files = render_all()
    stale = [
        p for p, text in files.items() if not p.exists() or p.read_text(encoding="utf-8") != text
    ]
    if args.check:
        for path in stale:
            print(f"stale: {path.relative_to(ROOT)}")
        if stale:
            print("Run: uv run python tools/l10n/extract.py")
            return 1
        return 0
    LANG.mkdir(exist_ok=True)
    for path in stale:
        path.write_text(files[path], encoding="utf-8", newline="\n")
        print(f"wrote {path.relative_to(ROOT)}")
    template_count = len(parse_po(files[TEMPLATE])) - 1
    print(f"{template_count} lines in the template")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
