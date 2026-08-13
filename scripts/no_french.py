#!/usr/bin/env python3
"""Fail if French text has crept back into the repository.

# Why this exists

The repository was written in French and converted to English in one pass. That
pass is worthless the moment someone reintroduces French, and nothing would
notice: French comments compile, French assertion messages pass, French prose
renders. Nobody re-reads a file that works.

So the convention gets a guard, like every other convention here that a test can
carry.

# ★ What this tool counts, and what it does not

It counts lines that **trigger the detector**, not lines that contain French. With a
deliberately short word list, it **under-counts by construction** — that is the
accepted trade-off below, not a defect.

So "0 remaining" means **0 detected**, never "no French left". A genuine zero comes
from reading each file, which is how the conversion pass was actually done; this
guard exists to stop French coming *back*, not to certify that it is gone.

Anyone quoting a number from here — including us in three months — should quote it as
"detected". The output says so on purpose.

# Why the word list is short

Only high-signal French function words, measured to produce **zero** false
positives on the already-translated crates. Deliberately excluded, and this is
not an oversight:

  on, plus, son, sans, par, sur   real English words
  ce                              would match `gpui-ce`, discussed in ADR 0023
  ne, il, ou                      too short, too many abbreviations

A guard that cries wolf gets switched off within a week. That is invariant 8,
applied to our own tooling: the cost of a missed French word is a follow-up
commit; the cost of a false positive is the guard itself.

One known collision, recorded so the next person does not rediscover it: `la`
matches the shell flag `-la` (`ls -la`), because `-` is a word boundary. Found
once, on a test fixture, and fixed by writing `ls -al` instead. If this recurs
often enough to be annoying, that is the signal to drop `la` from the list --
not to add exceptions one at a time.

Accented Latin letters are a second, independent signal — they carry no false
positives at all in an English repository.

# The negative control is permanent

`--self-test` feeds the detector known-French lines it must flag and known-English
lines it must spare, and it runs before every repository scan. A measuring device
that has never been seen to fail has not been verified.

Four ways of breaking this file were tried, and each one must turn the self-test
red — that is what makes the control a control rather than a decoration:

  1. empty FRENCH_WORDS
  2. disable ACCENT_RE
  3. drop re.IGNORECASE
  4. make is_allowed() return True for everything

Two of those found real holes on the first attempt. Case sensitivity meant a
sentence-initial capital walked straight past the guard. And no sample exercised
ACCENT_RE at all — every French line in the list happened to also contain a listed
word, so the accent branch could have been dead code and the self-test would still
have been green. Hence the two samples marked ONLY: each carries exactly one
signal, so a hole in that signal cannot hide behind the others.

# The one dated exception: filenames not yet renamed

2026-08-13. The 29 ADRs under `docs/adr/` and the probe reports under `docs/sondes/`
still have French filenames, because their translation is a separate pass (see the
debt in `AGENTS.md`). Every file that links to one therefore carries French inside a
path — `docs/adr/0016-interception-avant-disque-validee.md` — and there is nothing the
linking file can do about it.

So `PENDING_FILENAME_RE` strips those paths from a line before scanning it. This is
narrow on purpose: it removes a **path**, never prose, and it recognises only the two
directories whose renaming is owed. The day the ADRs are renamed, the regex stops
matching anything and can be deleted — the exception erases itself rather than
becoming permanent.

It carries its own negative control, in both directions: a line with a French link
*and* French prose must still be flagged, and a line that is only a French link must
not be. Without the first of those, this exception would be a way to smuggle French
past the guard by mentioning an ADR on the same line.
"""

from __future__ import annotations

import pathlib
import re
import sys

# High-signal French function words. See the module docstring for what is left out.
FRENCH_WORDS = """
    le la les des une est sont dans pour avec qui que cette donc mais aux ses
    leur doit doivent etre avoir peut faut nous vous alors quand meme chaque
    aucun toute tous celui cela ainsi entre sous vers apres avant deja aussi
    tres bien fait pas cote plutot parce afin lorsque comme tandis
    francais anglais
""".split()

# ★ Case-insensitive, and the self-test is why. The first version was
# case-sensitive and missed "Le domaine s'ecrit en francais." — a sentence-initial
# capital was enough to walk straight past the guard. It was caught on the very
# first run of --self-test, before the guard was trusted for anything.
WORD_RE = re.compile(r"\b(" + "|".join(FRENCH_WORDS) + r")\b", re.IGNORECASE)
ACCENT_RE = re.compile(r"[àâäçéèêëîïôöùûüÿœæÀÂÄÇÉÈÊËÎÏÔÖÙÛÜŸŒÆ]")

# ★ Paths of documents whose translation pass is still owed (2026-08-13). Stripped from
# a line before scanning, because a file that links to an ADR cannot rename it. This
# matches a PATH and never prose — see the module docstring for why that distinction is
# the whole safety of the exception, and for the two-way negative control on it.
# The `docs/` prefix is optional because documents INSIDE docs/ link relatively
# (`adr/0023-...md`), and those are the same unrenamed filenames.
PENDING_FILENAME_RE = re.compile(r"(?:docs/)?(?:adr|sondes)/[0-9][0-9A-Za-z._-]*\.md")

SCANNED_SUFFIXES = {".rs", ".md", ".toml", ".yml", ".yaml", ".sql"}
SCANNED_NAMES = {"justfile"}

SKIPPED_DIRS = {"target", ".git", "node_modules", ".jj"}

# Lines allowed to contain French, each with the reason it is unavoidable and, where the
# reason only applies to one document, the file it is confined to.
#
# Keep this list short: every entry is a hole in the guard. Prefer a scoped entry to an
# unscoped one — an unscoped needle excuses that string EVERYWHERE, including in the
# self-test fixtures, which turns the guard's own negative control green by accident. That
# happened on the last entry below, and the self-test caught it on the first run.
ALLOWED: list[tuple[str, str | None, str]] = [
    # (substring that must appear in the line, file it is confined to or None, why)
    (
        "scripts/no_french.py",
        None,
        "this file necessarily contains the words it looks for",
    ),
    ("FSL-1.1-MIT", None, "a licence identifier"),
    ("but agent setup", None, "a third-party command name"),
    (
        "Le domaine s'ecrit en francais.",
        "AGENTS.md",
        "AGENTS.md quotes this exact sample three times when telling the story of the "
        "eighth case, and it IS one of the self-test fixtures below. Confined to that one "
        "file on purpose: unscoped, it excused the fixture too, and the self-test went red "
        "to say so. The needle is the full sentence, so the hole is one sentence wide.",
    ),
]


def is_allowed(line: str, path: pathlib.Path) -> bool:
    if path.name == "no_french.py":
        return True
    return any(
        needle in line and (scope is None or path.name == scope)
        for needle, scope, _ in ALLOWED
    )


def scan_text(text: str, path: pathlib.Path) -> list[tuple[int, str, str]]:
    """Return (line number, what was found, the line) for every offending line."""
    found = []
    for number, line in enumerate(text.split("\n"), 1):
        if is_allowed(line, path):
            continue
        # A not-yet-renamed ADR or probe filename is a path, not prose. Whatever is
        # left of the line is still scanned in full, which is what keeps this from
        # being a way to smuggle French in by mentioning an ADR alongside it.
        scanned_line = PENDING_FILENAME_RE.sub("", line)
        words = sorted(set(WORD_RE.findall(scanned_line)))
        accents = sorted(set(ACCENT_RE.findall(scanned_line)))
        if words or accents:
            hit = ", ".join(words + accents)
            found.append((number, hit, line.strip()[:100]))
    return found


def files_to_scan(root: pathlib.Path):
    for path in sorted(root.rglob("*")):
        if any(part in SKIPPED_DIRS for part in path.parts):
            continue
        if not path.is_file():
            continue
        if path.suffix in SCANNED_SUFFIXES or path.name in SCANNED_NAMES:
            yield path


def self_test() -> bool:
    """★ The negative control, run on every invocation.

    A detector that has never been observed to fail has not been verified. These
    samples are the ones the real pass actually produced, not invented ones.
    """
    must_flag = [
        "//! Le registre est le point de passage unique des ecritures.",
        '    assert!(ok, "la lecture doit entrer dans le read-set");',
        "Le domaine s'ecrit en francais.",
        "//! La sequence est locale au projet, jamais globale.",
        "- **Cible primaire** : GitLab self-hosted, parce que c'est la cible.",
        # ★ Each of the next two carries exactly ONE signal, so a hole in that signal
        # cannot hide behind the others. The accent line was added after a negative
        # control found that disabling ACCENT_RE broke nothing the self-test watched:
        # every sample above happens to also contain a listed word.
        "/// Mesuré on macOS, Xcode 26.6.",  # accent ONLY — no listed word at all
        "Cette approach is too expensive.",  # one listed word ONLY — no accent
        # ★ The filename exception, direction 1: stripping the path must not also
        # excuse the prose around it. Without this sample, PENDING_FILENAME_RE would
        # be a way to smuggle a French sentence past the guard by citing an ADR on
        # the same line.
        "Detail dans [ADR 0016](docs/adr/0016-interception-avant-disque-validee.md).",
    ]
    must_not_flag = [
        "//! The registry is the single point of passage for writes.",
        '    assert!(ok, "the read must enter the read-set");',
        "/// A path outside the project is refused.",
        "//! `gpui-ce` remains the documented escape hatch.",
        "- The default is on, and nothing more is needed.",
        "/// Its writes are out-of-band — caught, but never admitted.",
        # ★ The filename exception, direction 2: an English line whose only French is
        # inside a path the linking file cannot rename must pass. This is the case
        # that made the exception necessary, and it is dated in the docstring.
        "Detail and method in [ADR 0016](docs/adr/0016-interception-avant-disque-validee.md).",
        "See [probe 3](docs/sondes/2026-08-12-postooluse.md) for what the hook reports.",
        # The relative form, as written by documents that live inside docs/ themselves.
        "An interface receives only that ([ADR 0023](adr/0023-gpui-amont-pour-la-gui.md)).",
    ]
    fake = pathlib.Path("fake.rs")
    ok = True
    for sample in must_flag:
        if not scan_text(sample, fake):
            print(f"  SELF-TEST FAILED: French not detected in: {sample[:60]}")
            ok = False
    for sample in must_not_flag:
        hits = scan_text(sample, fake)
        if hits:
            print(f"  SELF-TEST FAILED: English flagged as French: {sample[:60]}")
            print(f"                    matched: {hits[0][1]}")
            ok = False
    return ok


def main() -> int:
    if "--self-test" in sys.argv:
        if self_test():
            print("SELF-TEST: GREEN — the detector catches French and spares English")
            return 0
        print("SELF-TEST: RED — the detector is not trustworthy, fix it before trusting a pass")
        return 1

    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")

    # The guard checks itself before checking anything else.
    if not self_test():
        print("SELF-TEST: RED — refusing to report on the repository with a broken detector")
        return 1

    offenders: dict[pathlib.Path, list[tuple[int, str, str]]] = {}
    scanned = 0
    for path in files_to_scan(root):
        scanned += 1
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        hits = scan_text(text, path)
        if hits:
            offenders[path] = hits

    total = sum(len(v) for v in offenders.values())
    if not offenders:
        print(
            f"LANGUAGE: GREEN — 0 lines DETECTED in {scanned} files.\n"
            "  Reminder: this counts detector hits, not French. The word list is short\n"
            "  on purpose, so it under-counts. A real zero comes from reading the files."
        )
        return 0

    print(
        f"LANGUAGE: RED — {total} lines DETECTED in {len(offenders)} of {scanned} files\n"
        f"  (detector hits, not a French line count — the list under-counts by design)\n"
    )
    for path, hits in offenders.items():
        print(f"{path}  ({len(hits)})")
        for number, hit, line in hits[:5]:
            print(f"  {number:5}  [{hit}]  {line}")
        if len(hits) > 5:
            print(f"        … {len(hits) - 5} more")
    print(
        "\nThe repository is English-only. If a line genuinely must contain French,\n"
        "add it to ALLOWED in scripts/no_french.py with the reason — every entry\n"
        "there is a hole in the guard, so keep the list short."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
