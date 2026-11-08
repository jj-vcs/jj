"""Manage changelog notes in changelog/ and compile them into CHANGELOG.md.

Each pull request adds a note file to changelog/<type>/ instead of editing
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
import json
import re
import subprocess
import sys
import textwrap
import unicodedata
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path

import mdformat

# uv installs the project in editable mode, so this is the source checkout:
# <repo>/changelog/changelog.py.
REPO_ROOT = Path(__file__).resolve().parents[1]
NOTES_DIR = REPO_ROOT / "changelog"
CHANGELOG = REPO_ROOT / "CHANGELOG.md"
GITHUB_REPO = "jj-vcs/jj"
REPO_URL = f"https://github.com/{GITHUB_REPO}"
# Contributors are the authors of the commits from the previous version's tag
# to this branch on GitHub.
CONTRIBUTORS_HEAD = "main"
CONTRIBUTORS_INTRO = "Thanks to the people who made this release happen!"
# The version to pass to `release` to add notes as unreleased changes.
UNRELEASED = "Unreleased"
# Lines in CHANGELOG.md around the note saying where unreleased changes are,
# which `remove-unreleased-note` removes when building the docs.
UNRELEASED_NOTE_BEGIN = "<!-- BEGIN UNRELEASED NOTE -->"
UNRELEASED_NOTE_END = "<!-- END UNRELEASED NOTE -->"

# Notes are wrapped to 78 columns so that, once indented by the two-column
# bullet prefix in CHANGELOG.md, they fit in 80 columns.
WRAP_WIDTH = 78

# Note type -> CHANGELOG.md section, in the order `new` offers them. The notes
# of each type are in the changelog/<type>/ directory.
TYPE_SECTIONS = {
    "feature": "New features",
    "fix": "Fixed bugs",
    "breaking": "Breaking changes",
    "deprecation": "Deprecations",
    "packaging": "Packaging changes",
    "security": "Security fixes",
}
# "Release highlights" are added by hand during a release.
# "Contributors" is added in compile_section.
SECTION_ORDER = (
    "Security fixes",
    "Breaking changes",
    "Deprecations",
    "New features",
    "Fixed bugs",
    "Packaging changes",
)

TYPE_DIRS = ", ".join(f"changelog/{note_type}/" for note_type in TYPE_SECTIONS)

VERSION_RE = re.compile(r"\d+\.\d+\.\d+")
VERSION_HEADING_RE = re.compile(
    rf"## \[({VERSION_RE.pattern})\] - \d{{4}}-\d{{2}}-\d{{2}}"
)
VERSION_LINK_RE = re.compile(rf"\[{VERSION_RE.pattern}\]: ")
LEADING_BULLET_RE = re.compile(r"\A[*+-][ \t]+")
NOTE_NAME_RE = re.compile(r"[a-z0-9][a-z0-9-]*")


class ChangelogError(Exception):
    pass


@dataclass(frozen=True)
class Note:
    path: Path
    type: str
    body: str


def format_markdown(text: str) -> str:
    """Normalizes Markdown text the way notes are stored."""
    text = LEADING_BULLET_RE.sub("", text.strip())
    return mdformat.text(text, options={"wrap": WRAP_WIDTH}).rstrip("\n")


def parse_note(path: Path) -> Note:
    """Parses and validates a note file. Its directory is the note type."""
    note_type = path.parent.name
    if path.parent.parent != NOTES_DIR or note_type not in TYPE_SECTIONS:
        raise ChangelogError(
            f"{path}: changelog notes must be in one of these directories: {TYPE_DIRS}"
        )
    if path.suffix != ".md" or not path.is_file():
        raise ChangelogError(f"{path}: changelog notes must be `.md` files")
    body = path.read_text(encoding="utf-8").strip("\n")
    if not body:
        raise ChangelogError(f"{path}: changelog note is empty")
    if LEADING_BULLET_RE.match(body):
        raise ChangelogError(
            f"{path}: changelog note must not start with a list bullet; one is "
            "added when compiling CHANGELOG.md"
        )
    return Note(path=path, type=note_type, body=body)


def note_paths() -> list[Path]:
    """Returns the notes and any files that look like misplaced notes.

    These are the files in the type directories (except the `.gitkeep` files
    that keep the directories in the repository), and the Markdown files
    anywhere else in changelog/ except the README.
    """
    if not NOTES_DIR.is_dir():
        return []
    paths = {p for p in NOTES_DIR.rglob("*.md") if p != NOTES_DIR / "README.md"}
    for note_type in TYPE_SECTIONS:
        type_dir = NOTES_DIR / note_type
        if type_dir.is_dir():
            paths.update(p for p in type_dir.iterdir() if p.name != ".gitkeep")
    return sorted(paths)


def parse_notes(paths: Sequence[Path]) -> tuple[list[Note], list[str]]:
    """Parses the notes. Returns the valid notes and an error per invalid file."""
    notes = []
    errors = []
    for path in paths:
        try:
            notes.append(parse_note(path))
        except ChangelogError as e:
            errors.append(str(e))
    return notes, errors


def load_notes(paths: Sequence[Path]) -> list[Note]:
    """Parses the notes, raising an error if any are invalid."""
    notes, errors = parse_notes(paths)
    if errors:
        raise ChangelogError("\n".join(errors))
    return notes


def bullet(text: str) -> str:
    """Formats text as a list item, indenting every non-blank line after the first.

    Blank lines are left empty rather than indented, so they have no trailing
    whitespace.
    """
    return "* " + textwrap.indent(text, "  ").removeprefix("  ")


def compile_section(
    version: str,
    date: str | None,
    notes: Sequence[Note],
    contributors: Sequence[str] = (),
) -> str:
    """Renders a CHANGELOG.md version section, ending with a blank line.

    The heading has no date if `date` is None. Empty sections are omitted.
    `contributors` are the entries of the Contributors section.
    """
    entries: dict[str, list[str]] = {section: [] for section in SECTION_ORDER}
    for note in sorted(notes, key=lambda n: n.path.name):
        entries[TYPE_SECTIONS[note.type]].append(note.body)

    heading = f"## [{version}]" if date is None else f"## [{version}] - {date}"
    out = [heading, ""]
    for section in SECTION_ORDER:
        if not entries[section]:
            continue
        out += [f"### {section}", ""]
        for entry in entries[section]:
            out += [bullet(entry), ""]
    if contributors:
        out += ["### Contributors", "", CONTRIBUTORS_INTRO, ""]
        out += [bullet(name) for name in contributors] + [""]
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
    notes, errors = parse_notes(note_paths())
    for note in notes:
        formatted = format_markdown(note.body)
        if formatted != note.body:
            errors.append(
                f"{note.path}: changelog note is not formatted; it should be:\n"
                + textwrap.indent(formatted, "    ")
            )
    changelog = CHANGELOG.read_text(encoding="utf-8")
    errors.extend(check_changelog_headings(changelog))
    try:
        find_unreleased_note(changelog.splitlines())
    except ChangelogError as e:
        errors.append(str(e))
    return errors


@dataclass(frozen=True)
class ReleasePosition:
    heading_index: int
    link_index: int
    previous_version: str


def release_position(lines: Sequence[str], version: str) -> ReleasePosition:
    """Finds where a new version's section and link go in CHANGELOG.md."""
    heading_index = previous_version = None
    for i, line in enumerate(lines):
        m = VERSION_HEADING_RE.match(line)
        if m:
            heading_index, previous_version = i, m.group(1)
            break
    if heading_index is None or previous_version is None:
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
    return ReleasePosition(heading_index, link_index, previous_version)


def fetch_compare_commits(base: str, head: str) -> list[dict]:
    """Returns the commits in `base...head` on GitHub, using the `gh` CLI."""
    command = [
        "gh",
        "api",
        f"/repos/{GITHUB_REPO}/compare/{base}...{head}",
        "--paginate",
        "--jq",
        ".commits[]",
    ]
    try:
        result = subprocess.run(command, check=False, capture_output=True, text=True)
    except FileNotFoundError:
        raise ChangelogError(
            "listing contributors requires the GitHub CLI (`gh`); install it and "
            "run `gh auth login`, or pass --no-contributors"
        )
    if result.returncode != 0:
        raise ChangelogError(
            f"`{' '.join(command)}` failed:\n{result.stderr.strip()}\n"
            "Pass --no-contributors to skip listing contributors."
        )
    # `gh` prints each commit as JSON on its own line.
    return [json.loads(line) for line in result.stdout.splitlines()]


def format_contributors(commits: Sequence[dict]) -> list[str]:
    """Returns `Name (@login)` entries for the commits' authors, sorted.

    Bots are skipped. Authors are deduplicated by GitHub account, or by email
    address for commits that aren't linked to a GitHub account (which are
    listed by name only). The first commit's author name is used.
    """
    contributors: dict[tuple[str, str], str] = {}
    for commit in commits:
        login = (commit.get("author") or {}).get("login")
        if login is not None and login.endswith("[bot]"):
            continue
        author = commit["commit"]["author"]
        key = ("login", login.lower()) if login else ("email", author["email"].lower())
        contributors.setdefault(
            key, f"{author['name']} (@{login})" if login else author["name"]
        )
    return sorted(contributors.values(), key=contributor_name_sort_key)


def contributor_name_sort_key(text: str) -> tuple[str, str]:
    """Sorts case-insensitively and ignoring accents, so "Gaëtan" < "Gasper"."""
    decomposed = unicodedata.normalize("NFKD", text.casefold())
    return ("".join(c for c in decomposed if not unicodedata.combining(c)), text)


def today() -> str:
    return datetime.date.today().isoformat()


def is_unreleased(version: str) -> bool:
    return version.lower() == UNRELEASED.lower()


@dataclass(frozen=True)
class Released:
    # The notes that were compiled and deleted.
    notes: list[Path]
    # Whether the Contributors section was filled in.
    contributors_listed: bool


def release(version: str, date: str | None, contributors: bool | None = None) -> Released:
    """Compiles notes into CHANGELOG.md and deletes them.

    If `version` is "Unreleased", the notes are added as an undated
    `## [Unreleased]` section, which may be empty. This is used to show
    unreleased changes in the prerelease docs.

    If `contributors` is true, the Contributors section lists the authors of
    the commits since the previous version on GitHub. It defaults to true
    unless `version` is "Unreleased".
    """
    unreleased = is_unreleased(version)
    if unreleased:
        if date is not None:
            raise ChangelogError(f"a date can't be given for {UNRELEASED}")
        if contributors:
            raise ChangelogError(f"contributors can't be listed for {UNRELEASED}")
        version = UNRELEASED
    elif not VERSION_RE.fullmatch(version):
        raise ChangelogError(
            f"version must look like X.Y.Z or be {UNRELEASED}, got {version!r}"
        )
    else:
        date = date or today()
    if contributors is None:
        contributors = not unreleased

    paths = note_paths()
    if not paths and not unreleased:
        raise ChangelogError(f"no changelog notes in {NOTES_DIR}")
    notes = load_notes(paths)
    lines = CHANGELOG.read_text(encoding="utf-8").splitlines(keepends=True)
    pos = release_position(lines, version)
    commits = (
        fetch_compare_commits(f"v{pos.previous_version}", CONTRIBUTORS_HEAD)
        if contributors
        else []
    )
    contributor_entries = format_contributors(commits)
    head = "HEAD" if unreleased else f"v{version}"
    link = f"[{version}]: {REPO_URL}/compare/v{pos.previous_version}...{head}\n"
    # The link goes below the section, so insert it first.
    lines.insert(pos.link_index, link)
    section = compile_section(version, date, notes, contributor_entries)
    lines.insert(pos.heading_index, section)
    CHANGELOG.write_text("".join(lines), encoding="utf-8")
    for path in paths:
        path.unlink()
    return Released(notes=paths, contributors_listed=bool(contributor_entries))


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
    input_fn: Callable[[str], str], print_fn: Callable[[str], None], message: str
) -> str:
    while True:
        print_fn(f"{message} Finish with an empty line:")
        text, eof = read_paragraph(input_fn)
        if text.strip():
            return format_markdown(text)
        if eof:
            raise ChangelogError("aborted")
        print_fn("This is required.")


def prompt_type(input_fn: Callable[[str], str], print_fn: Callable[[str], None]) -> str:
    """Returns the chosen note type."""
    choices = list(TYPE_SECTIONS.items())
    print_fn("What kind of change is this?")
    for i, (_, label) in enumerate(choices, start=1):
        print_fn(f"  {i}) {label}")
    while True:
        try:
            answer = input_fn(f"Choice [1-{len(choices)}]: ").strip()
        except EOFError:
            raise ChangelogError("aborted") from None
        if answer.isdigit() and 1 <= int(answer) <= len(choices):
            return choices[int(answer) - 1][0]
        print_fn(f"Please enter a number from 1 to {len(choices)}.")


def slugify(text: str) -> str:
    # Drop link targets so that URLs don't end up in file names.
    text = re.sub(r"\]\([^)]*\)", "]", text)
    words = re.findall(r"[a-z0-9]+", text.lower())
    return "-".join(words[:6]) or "note"


def unique_note_path(note_type: str, name: str) -> Path:
    path = NOTES_DIR / note_type / f"{name}.md"
    n = 2
    while path.exists():
        path = path.with_stem(f"{name}-{n}")
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

    note_type = prompt_type(input_fn, print_fn)
    body = prompt_text(input_fn, print_fn, "Describe the change for the changelog.")

    path = unique_note_path(note_type, name or slugify(body))
    path.write_text(f"{body}\n", encoding="utf-8")
    return path


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
        "--date", help=f"release date (default: today; not allowed with {UNRELEASED})"
    )
    release_parser.add_argument(
        "--contributors",
        action=argparse.BooleanOptionalAction,
        help="list the authors of the commits since the previous version on "
        f"GitHub's {CONTRIBUTORS_HEAD} branch, using the GitHub CLI (`gh`) "
        "(default: on, except for Unreleased)",
    )

    subparsers.add_parser(
        "remove-unreleased-note",
        help=f"remove the note between the {UNRELEASED_NOTE_BEGIN} and "
        f"{UNRELEASED_NOTE_END} lines from CHANGELOG.md, for building the docs",
    )

    args = parser.parse_args(argv)
    try:
        if args.command == "new":
            path = new_note(args.name).relative_to(REPO_ROOT)
            print(f"Created {path}. Review it and add it to your commit.")
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
            released = release(args.version, args.date, args.contributors)
            print(
                f"Added {args.version} to {CHANGELOG.name} from "
                f"{len(released.notes)} notes and deleted them."
            )
            if not is_unreleased(args.version):
                if released.contributors_listed:
                    sections = 'a "Release highlights" section (if relevant)'
                else:
                    sections = (
                        '"Release highlights" (if relevant) and "Contributors" sections'
                    )
                print(f"Add {sections} to it; see docs/releasing.md.")
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
