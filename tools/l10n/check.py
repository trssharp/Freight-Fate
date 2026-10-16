"""Check every language file in ``lang/`` for mistakes the game cannot fix.

    uv run python tools/l10n/check.py           # fail on any problem
    uv run python tools/l10n/check.py --stats   # also print coverage

A translated entry must keep every placeholder of its English line and name
no other, keep every number, and keep the key names the help reads out
(Enter, Escape, F1 ...). A plural choice must give as many forms as the
language has. The game skips an entry it cannot use and speaks the English
instead, so none of these break the game; they break the translation.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from extract import LANG, LANGUAGES, language_codes  # noqa: E402
from po import PoError, parse_po  # noqa: E402

HOLE = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)(?::([^{}]*))?\}")
NUMBER = re.compile(r"\d+")
# Key names as the help and the key settings speak them. "Shift" is left
# out: it is also the gearshift verb ("Shift up").
KEY_NAMES = re.compile(
    r"(?<![A-Za-z])(Enter|Escape|Backspace|Page Up|Page Down|Tab|Control|Alt|F(?:1[0-2]|[1-9]))(?![A-Za-z])"
)
PLURAL_FORMS = {"zh_CN": 1, "ja": 1, "ru": 3, "pl": 3}


def _without_holes(text: str) -> str:
    return HOLE.sub(" ", text)


def check_entry(code: str, msgid: str, msgstr: str) -> list[str]:
    problems = []
    source_holes = {m.group(1) for m in HOLE.finditer(msgid) if m.group(2) is None}
    translated = list(HOLE.finditer(msgstr))
    used = {m.group(1) for m in translated}
    for name in sorted(used - source_holes):
        problems.append(f"names {{{name}}}, which the English line does not have")
    # {s_ending} is the English "s"; a translation leaves it out.
    for name in sorted(source_holes - used - {"s_ending"}):
        problems.append(f"drops {{{name}}}")
    depth = 0
    for ch in msgstr:
        depth += {"{": 1, "}": -1}.get(ch, 0)
        if depth > 1 or depth < 0:
            problems.append("has a brace inside a placeholder, or an unmatched brace")
            break
    else:
        if depth:
            problems.append("has an unmatched brace")
    want = PLURAL_FORMS.get(code, 2)
    for m in translated:
        if m.group(2) is not None:
            forms = m.group(2).split("|")
            if len(forms) != want and not (want == 1 and len(forms) >= 1):
                problems.append(f"{{{m.group(1)}:...}} gives {len(forms)} forms; {code} has {want}")
    source_numbers = sorted(NUMBER.findall(_without_holes(msgid)))
    target_numbers = NUMBER.findall(_without_holes(msgstr))
    for number in source_numbers:
        if number in target_numbers:
            target_numbers.remove(number)
        else:
            problems.append(f"loses the number {number}")
    target_keys = KEY_NAMES.findall(msgstr)
    for key in KEY_NAMES.findall(msgid):
        if key in target_keys:
            target_keys.remove(key)
        else:
            problems.append(f"translates or drops the key name {key}")
    return problems


def check_file(code: str, path: Path) -> tuple[list[str], int, int]:
    try:
        entries = parse_po(path.read_text(encoding="utf-8"))
    except PoError as err:
        return [f"{path.name}: {err}"], 0, 0
    problems = []
    total = translated = 0
    for entry in entries:
        if not entry.msgid or entry.obsolete:
            continue
        total += 1
        if not entry.msgstr or entry.fuzzy:
            continue
        translated += 1
        for problem in check_entry(code, entry.msgid, entry.msgstr):
            problems.append(f"{path.name}:{entry.line}: {problem}: {entry.msgid[:70]!r}")
    return problems, total, translated


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--stats", action="store_true", help="print how much is translated")
    args = parser.parse_args(argv)
    failed = False
    for code in language_codes():
        name = LANGUAGES[code][0] if code in LANGUAGES else code
        path = LANG / f"{code}.po"
        if not path.exists():
            print(f"missing: lang/{code}.po")
            failed = True
            continue
        problems, total, translated = check_file(code, path)
        for problem in problems:
            print(problem)
        failed |= bool(problems)
        if args.stats:
            share = 100 * translated / total if total else 0
            print(f"{name} ({code}): {translated} of {total} lines, {share:.0f} percent")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
