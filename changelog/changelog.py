"""Manage changelog notes in changelog/ and compile them into CHANGELOG.md.

Each pull request adds a note file to changelog/ instead of editing
CHANGELOG.md. At release time, the notes are compiled into a new version
section of CHANGELOG.md and deleted.

Commands:
  new                     interactively create a changelog note (for contributors)
  check                   validate the notes and CHANGELOG.md (run in CI)
  preview                 print the section a release would add to CHANGELOG.md
  release                 compile the notes into CHANGELOG.md and delete them
  remove-unreleased-note  remove the note saying where unreleased changes are
                          from CHANGELOG.md (when building the docs)

Run with `uv run changelog <command>`. The command is defined in pyproject.toml.
"""

from __future__ import annotations

import argparse
import datetime
import re
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import mdformat
import yaml

# uv installs the project in editable mode, so this is the source checkout:
# <repo>/changelog/changelog.py.
REPO_ROOT = Path(__file__).resolve().parents[1]
NOTES_DIR = REPO_ROOT / "changelog"
CHANGELOG = REPO_ROOT / "CHANGELOG.md"
REPO_URL = "https://github.com/jj-vcs/jj"
# The version to pass to `release` to add notes as unreleased changes.
UNRELEASED = "Unreleased"
# Lines in CHANGELOG.md around the note saying where unreleased changes are,
# which `remove-unreleased-note` removes when building the docs.
UNRELEASED_NOTE_BEGIN = "<!-- BEGIN UNRELEASED NOTE -->"
UNRELEASED_NOTE_END = "<!-- END UNRELEASED NOTE -->"

# Notes are wrapped to 78 columns so that, once indented by the two-column
# bullet prefix in CHANGELOG.md, they fit in 80 columns.
WRAP_WIDTH = 78

# Note type -> CHANGELOG.md section.
TYPE_SECTIONS = {
    "feature": "New features",
    "fix": "Fixed bugs",
    "deprecation": "Deprecations",
    "packaging": "Packaging changes",
    "security": "Security fixes",
}
BREAKING_SECTION = "Breaking changes"
# "Release highlights" and "Contributors" are added by hand during a release.
SECTION_ORDER = (
    "Security fixes",
    BREAKING_SECTION,
    "Deprecations",
    "New features",
    "Fixed bugs",
    "Packaging changes",
)

# Shown by `new`, in this order. `None` means a breaking change with no entry
# in any other section.
TYPE_CHOICES = (
    ("feature", "New feature"),
    ("fix", "Bug fix"),
    ("deprecation", "Deprecation"),
    ("packaging", "Packaging change"),
    ("security", "Security fix"),
    (None, "Breaking change only (no feature/fix entry)"),
)

NOTE_KEYS = {"type", "breaking_note"}
# Files in changelog/ that aren't notes: the README and the Python files that
# implement this tool.
IGNORED_NOTE_FILES = {"README.md"}
IGNORED_NOTE_SUFFIXES = {".py"}

VERSION_RE = re.compile(r"\d+\.\d+\.\d+")
VERSION_HEADING_RE = re.compile(r"## \[(\d+\.\d+\.\d+)\] - \d{4}-\d{2}-\d{2}")
VERSION_LINK_RE = re.compile(r"\[\d+\.\d+\.\d+\]: ")
LEADING_BULLET_RE = re.compile(r"\A[*+-][ \t]+")
NOTE_NAME_RE = re.compile(r"[a-z0-9][a-z0-9-]*")


class ChangelogError(Exception):
    pass


@dataclass(frozen=True)
class Note:
    path: Path
    type: str | None
    breaking_note: str | None
    body: str | None


def format_markdown(text: str) -> str:
    """Normalizes Markdown text the way notes are stored."""
    text = LEADING_BULLET_RE.sub("", text.strip())
    return mdformat.text(text, options={"wrap": WRAP_WIDTH}).rstrip("\n")


def parse_note(path: Path) -> Note:
    """Parses and validates the structure of a note file."""
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    if not lines or lines[0] != "---":
        raise ChangelogError(f"{path}: must start with a `---` front matter line")
    try:
        end = lines.index("---", 1)
    except ValueError:
        raise ChangelogError(f"{path}: front matter is not closed with `---`")
    try:
        front = yaml.safe_load("\n".join(lines[1:end]))
    except yaml.YAMLError as e:
        raise ChangelogError(f"{path}: invalid YAML front matter: {e}")
    if front is None:
        front = {}
    if not isinstance(front, dict):
        raise ChangelogError(f"{path}: front matter must be a YAML mapping")
    unknown = sorted(set(map(str, front)) - NOTE_KEYS)
    if unknown:
        raise ChangelogError(
            f"{path}: unknown front matter keys: {', '.join(unknown)} "
            f"(allowed: {', '.join(sorted(NOTE_KEYS))})"
        )

    note_type = front.get("type")
    if note_type is not None and note_type not in TYPE_SECTIONS:
        raise ChangelogError(
            f"{path}: `type` must be one of: {', '.join(TYPE_SECTIONS)}"
        )
    breaking_note = front.get("breaking_note")
    if breaking_note is not None:
        if not isinstance(breaking_note, str):
            raise ChangelogError(f"{path}: `breaking_note` must be a string")
        breaking_note = breaking_note.strip("\n") or None
    body = "\n".join(lines[end + 1 :]).strip("\n") or None

    if note_type is None and body is not None:
        raise ChangelogError(f"{path}: an entry body requires a `type`")
    if note_type is not None and body is None:
        raise ChangelogError(f"{path}: `type: {note_type}` requires an entry body")
    if note_type is None and breaking_note is None:
        raise ChangelogError(
            f"{path}: needs a `type` and an entry body, a `breaking_note`, or both"
        )
    for name, value in (("entry body", body), ("breaking_note", breaking_note)):
        if value is not None and LEADING_BULLET_RE.match(value):
            raise ChangelogError(
                f"{path}: {name} must not start with a list bullet; one is "
                "added when compiling CHANGELOG.md"
            )
    return Note(path=path, type=note_type, breaking_note=breaking_note, body=body)


def is_ignored(path: Path) -> bool:
    return path.name in IGNORED_NOTE_FILES or path.suffix in IGNORED_NOTE_SUFFIXES


def note_paths() -> list[Path]:
    if not NOTES_DIR.is_dir():
        return []
    return sorted(p for p in NOTES_DIR.iterdir() if p.is_file() and not is_ignored(p))


def load_notes(paths: Sequence[Path]) -> list[Note]:
    errors = []
    notes = []
    for path in paths:
        if path.suffix != ".md":
            errors.append(f"{path}: changelog notes must have a `.md` extension")
            continue
        try:
            notes.append(parse_note(path))
        except ChangelogError as e:
            errors.append(str(e))
    if errors:
        raise ChangelogError("\n".join(errors))
    return notes


def render_note(
    note_type: str | None, breaking_note: str | None, body: str | None
) -> str:
    """Renders the contents of a note file."""
    lines = ["---"]
    if note_type is not None:
        lines.append(f"type: {note_type}")
    if breaking_note is not None:
        lines.append("breaking_note: |")
        lines.extend(f"  {line}" if line else "" for line in breaking_note.split("\n"))
    lines.append("---")
    if body is not None:
        lines.append(body)
    return "\n".join(lines) + "\n"


def bullet(text: str) -> str:
    first, *rest = text.split("\n")
    return "\n".join([f"* {first}"] + [f"  {line}" if line else "" for line in rest])


def compile_section(version: str, date: str | None, notes: Sequence[Note]) -> str:
    """Renders a CHANGELOG.md version section, ending with a blank line.

    The heading has no date if `date` is None. Empty sections are omitted.
    """
    entries: dict[str, list[str]] = {section: [] for section in SECTION_ORDER}
    for note in sorted(notes, key=lambda n: n.path.name):
        if note.breaking_note is not None:
            entries[BREAKING_SECTION].append(note.breaking_note)
        if note.type is not None:
            assert note.body is not None
            entries[TYPE_SECTIONS[note.type]].append(note.body)

    heading = f"## [{version}]" if date is None else f"## [{version}] - {date}"
    out = [heading, ""]
    for section in SECTION_ORDER:
        if not entries[section]:
            continue
        out += [f"### {section}", ""]
        for entry in entries[section]:
            out += [bullet(entry), ""]
    return "\n".join(out) + "\n"


def check_changelog_headings(text: str) -> list[str]:
    errors = []
    for lineno, line in enumerate(text.splitlines(), start=1):
        if line.startswith("## ") and not VERSION_HEADING_RE.fullmatch(line):
            errors.append(
                f"{CHANGELOG.name}:{lineno}: unexpected heading {line!r}; only "
                "`## [X.Y.Z] - YYYY-MM-DD` headings are allowed. Add a note "
                "to changelog/ instead of editing CHANGELOG.md."
            )
    return errors


def find_unreleased_note(lines: Sequence[str]) -> tuple[int, int]:
    """Returns the indexes of the lines that begin and end the unreleased note."""
    begins = [i for i, line in enumerate(lines) if line.strip() == UNRELEASED_NOTE_BEGIN]
    ends = [i for i, line in enumerate(lines) if line.strip() == UNRELEASED_NOTE_END]
    if len(begins) != 1 or len(ends) != 1 or begins[0] > ends[0]:
        raise ChangelogError(
            f"{CHANGELOG.name}: expected one `{UNRELEASED_NOTE_BEGIN}` line followed "
            f"by one `{UNRELEASED_NOTE_END}` line"
        )
    return begins[0], ends[0]


def remove_unreleased_note(changelog: str) -> str:
    """Removes the unreleased note, its markers, and the blank line after it."""
    lines = changelog.splitlines(keepends=True)
    begin, end = find_unreleased_note(lines)
    stop = end + 1
    if stop < len(lines) and not lines[stop].strip():
        stop += 1
    return "".join(lines[:begin] + lines[stop:])


def remove_unreleased_note_from_changelog() -> None:
    """Removes the unreleased note from CHANGELOG.md."""
    changelog = CHANGELOG.read_text(encoding="utf-8")
    CHANGELOG.write_text(remove_unreleased_note(changelog), encoding="utf-8")


def check() -> list[str]:
    """Returns a list of problems with the notes and CHANGELOG.md."""
    errors = []
    try:
        notes = load_notes(note_paths())
    except ChangelogError as e:
        errors.extend(str(e).split("\n"))
        notes = []
    for note in notes:
        for name, value in (("entry body", note.body), ("breaking_note", note.breaking_note)):
            if value is None:
                continue
            formatted = format_markdown(value)
            if formatted != value:
                errors.append(
                    f"{note.path}: {name} is not formatted; it should be:\n"
                    + "\n".join(f"    {line}" for line in formatted.split("\n"))
                )
    changelog = CHANGELOG.read_text(encoding="utf-8")
    errors.extend(check_changelog_headings(changelog))
    try:
        find_unreleased_note(changelog.splitlines())
    except ChangelogError as e:
        errors.append(str(e))
    return errors


def insert_release(changelog: str, version: str, section: str) -> str:
    """Inserts a version section and its comparison link into CHANGELOG.md."""
    lines = changelog.splitlines(keepends=True)
    heading_index = prev_version = None
    for i, line in enumerate(lines):
        m = VERSION_HEADING_RE.match(line)
        if m:
            heading_index, prev_version = i, m.group(1)
            break
    if heading_index is None:
        raise ChangelogError(
            f"{CHANGELOG.name}: no `## [X.Y.Z] - YYYY-MM-DD` heading to insert above"
        )
    if any(line.startswith(f"## [{version}]") for line in lines):
        raise ChangelogError(f"{CHANGELOG.name}: version {version} already exists")
    link_index = next(
        (i for i, line in enumerate(lines) if VERSION_LINK_RE.match(line)), None
    )
    if link_index is None:
        raise ChangelogError(
            f"{CHANGELOG.name}: no `[X.Y.Z]: <url>` link line to insert above"
        )
    if version == UNRELEASED:
        link = f"[{UNRELEASED}]: {REPO_URL}/compare/v{prev_version}...HEAD\n"
    else:
        link = f"[{version}]: {REPO_URL}/compare/v{prev_version}...v{version}\n"
    return "".join(
        lines[:heading_index]
        + [section]
        + lines[heading_index:link_index]
        + [link]
        + lines[link_index:]
    )


def today() -> str:
    return datetime.datetime.now(datetime.timezone.utc).date().isoformat()


def release(version: str, date: str | None) -> list[Path]:
    """Compiles notes into CHANGELOG.md, deletes them, and returns their paths.

    If `version` is "Unreleased", the notes are added as an undated
    `## [Unreleased]` section, which may be empty. This is used to show
    unreleased changes in the prerelease docs.
    """
    unreleased = version.lower() == UNRELEASED.lower()
    if unreleased:
        if date is not None:
            raise ChangelogError(f"a date can't be given for {UNRELEASED}")
        version = UNRELEASED
    elif VERSION_RE.fullmatch(version):
        date = date or today()
    else:
        raise ChangelogError(
            f"version must look like X.Y.Z or be {UNRELEASED}, got {version!r}"
        )
    paths = note_paths()
    if not paths and not unreleased:
        raise ChangelogError(f"no changelog notes in {NOTES_DIR}")
    notes = load_notes(paths)
    changelog = CHANGELOG.read_text(encoding="utf-8")
    section = compile_section(version, date, notes)
    updated = insert_release(changelog, version, section)
    CHANGELOG.write_text(updated, encoding="utf-8")
    for path in paths:
        path.unlink()
    return paths


def read_paragraph(input_fn: Callable[[str], str]) -> tuple[str, bool]:
    """Reads lines until an empty line. Returns the text and whether EOF was hit."""
    lines = []
    while True:
        try:
            line = input_fn("> ").rstrip()
        except EOFError:
            return "\n".join(lines), True
        if not line:
            return "\n".join(lines), False
        lines.append(line)


def prompt_text(
    input_fn: Callable[[str], str],
    print_fn: Callable[[str], None],
    message: str,
    required: bool,
) -> str | None:
    while True:
        print_fn(message)
        text, eof = read_paragraph(input_fn)
        if text.strip():
            return format_markdown(text)
        if not required:
            return None
        if eof:
            raise ChangelogError("aborted")
        print_fn("This is required.")


def prompt_type(
    input_fn: Callable[[str], str], print_fn: Callable[[str], None]
) -> tuple[str | None, bool]:
    """Returns the chosen note type and whether it is a breaking-only note."""
    print_fn("What kind of change is this?")
    for i, (_, label) in enumerate(TYPE_CHOICES, start=1):
        print_fn(f"  {i}) {label}")
    while True:
        try:
            answer = input_fn(f"Choice [1-{len(TYPE_CHOICES)}]: ").strip()
        except EOFError:
            raise ChangelogError("aborted")
        if answer.isdigit() and 1 <= int(answer) <= len(TYPE_CHOICES):
            note_type = TYPE_CHOICES[int(answer) - 1][0]
            return note_type, note_type is None
        print_fn(f"Please enter a number from 1 to {len(TYPE_CHOICES)}.")


def slugify(text: str) -> str:
    # Drop link targets so that URLs don't end up in file names.
    text = re.sub(r"\]\([^)]*\)", "]", text)
    words = re.findall(r"[a-z0-9]+", text.lower())
    return "-".join(words[:6]) or "note"


def unique_note_path(name: str) -> Path:
    path = NOTES_DIR / f"{name}.md"
    n = 2
    while path.exists():
        path = NOTES_DIR / f"{name}-{n}.md"
        n += 1
    return path


def new_note(
    name: str | None,
    input_fn: Callable[[str], str] = input,
    print_fn: Callable[[str], None] = print,
) -> Path:
    """Interactively creates a note file and returns its path."""
    if name is not None:
        name = name.removesuffix(".md")
        if not NOTE_NAME_RE.fullmatch(name):
            raise ChangelogError(
                f"note name {name!r} must be lowercase letters, digits, and dashes"
            )

    note_type, breaking_only = prompt_type(input_fn, print_fn)
    if breaking_only:
        breaking_note = prompt_text(
            input_fn,
            print_fn,
            "Describe the breaking change. Finish with an empty line:",
            required=True,
        )
        body = None
    else:
        breaking_note = prompt_text(
            input_fn,
            print_fn,
            "If this is a breaking change, describe what users need to know. "
            "Finish with an empty line (leave empty if not breaking):",
            required=False,
        )
        body = prompt_text(
            input_fn,
            print_fn,
            "Describe the change for the changelog. Finish with an empty line:",
            required=True,
        )

    path = unique_note_path(name or slugify(body or breaking_note or ""))
    NOTES_DIR.mkdir(exist_ok=True)
    path.write_text(render_note(note_type, breaking_note, body), encoding="utf-8")
    return path


def display_path(path: Path) -> str:
    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    new_parser = subparsers.add_parser("new", help="interactively create a note")
    new_parser.add_argument(
        "--name", help="file name for the note (default: derived from the text)"
    )

    subparsers.add_parser("check", help="validate the notes and CHANGELOG.md")

    preview_parser = subparsers.add_parser(
        "preview", help="print the section a release would add"
    )
    preview_parser.add_argument("--version", default=UNRELEASED)
    preview_parser.add_argument("--date", help="release date (default: none)")

    release_parser = subparsers.add_parser(
        "release", help="compile the notes into CHANGELOG.md and delete them"
    )
    release_parser.add_argument(
        "version",
        help=f"the new version, e.g. 0.46.0, or {UNRELEASED} to add the notes "
        "as unreleased changes (used to build the prerelease docs)",
    )
    release_parser.add_argument(
        "--date", help=f"release date (default: today, UTC; not allowed with {UNRELEASED})"
    )

    subparsers.add_parser(
        "remove-unreleased-note",
        help=f"remove the note between the {UNRELEASED_NOTE_BEGIN} and "
        f"{UNRELEASED_NOTE_END} lines from CHANGELOG.md, for building the docs",
    )

    args = parser.parse_args(argv)
    try:
        if args.command == "new":
            path = new_note(args.name)
            print(f"Created {display_path(path)}. Review it and add it to your commit.")
        elif args.command == "check":
            errors = check()
            for error in errors:
                print(error, file=sys.stderr)
            if errors:
                return 1
            print(f"{len(note_paths())} changelog notes OK")
        elif args.command == "preview":
            notes = load_notes(note_paths())
            sys.stdout.write(compile_section(args.version, args.date, notes))
        elif args.command == "release":
            paths = release(args.version, args.date)
            print(
                f"Added {args.version} to {CHANGELOG.name} from {len(paths)} notes "
                "and deleted them."
            )
            if args.version.lower() != UNRELEASED.lower():
                print(
                    'Add "Release highlights" (if relevant) and "Contributors" '
                    "sections to it; see docs/releasing.md."
                )
        elif args.command == "remove-unreleased-note":
            remove_unreleased_note_from_changelog()
            print(f"Removed the unreleased note from {CHANGELOG.name}.")
    except ChangelogError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\naborted", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    sys.exit(main())
