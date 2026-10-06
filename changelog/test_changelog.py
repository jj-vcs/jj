"""Tests for the changelog tool in changelog.py.

Run with `uv run python changelog/test_changelog.py`.
"""

from __future__ import annotations

import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

import changelog

UNRELEASED_NOTE = """\
<!-- BEGIN UNRELEASED NOTE -->
Unreleased changes are in changelog/.
<!-- END UNRELEASED NOTE -->

"""

CHANGELOG_TEXT = f"""\
# Changelog

Intro text.

{UNRELEASED_NOTE}\
## [0.2.0] - 2026-02-01

### Fixed bugs

* Old fix.

## [0.1.0] - 2026-01-01

### New features

* Old feature.

[0.2.0]: https://github.com/jj-vcs/jj/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jj-vcs/jj/releases/tag/v0.1.0
"""


def scripted_input(answers):
    """Returns an input() replacement that replays answers, then raises EOF."""
    answers = iter(answers)

    def input_fn(prompt):
        try:
            return next(answers)
        except StopIteration:
            raise EOFError

    return input_fn


class ChangelogTestCase(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.notes_dir = self.root / "changelog"
        for note_type in changelog.TYPE_SECTIONS:
            (self.notes_dir / note_type).mkdir(parents=True)
            (self.notes_dir / note_type / ".gitkeep").write_text("")
        self.changelog = self.root / "CHANGELOG.md"
        self.changelog.write_text(CHANGELOG_TEXT)
        for name, value in (
            ("REPO_ROOT", self.root),
            ("NOTES_DIR", self.notes_dir),
            ("CHANGELOG", self.changelog),
        ):
            patcher = mock.patch.object(changelog, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def write_note(self, name, text):
        path = self.notes_dir / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(text))
        return path

    def new(self, answers, name=None):
        return changelog.new_note(name, scripted_input(answers), lambda _: None)


class NewNoteTest(ChangelogTestCase):
    def test_feature_is_wrapped_and_normalized(self):
        path = self.new(
            [
                "1",
                "* `jj workspace add` supports   `--colocate`/`--no-colocate` flags to "
                "control whether a Git worktree is created alongside the workspace,",
                "see `jj workspace forget --help for details`.",
                "",
            ]
        )
        self.assertEqual(
            path, self.notes_dir / "feature" / "jj-workspace-add-supports-colocate-no.md"
        )
        self.assertEqual(
            path.read_text(),
            "`jj workspace add` supports `--colocate`/`--no-colocate` flags to control\n"
            "whether a Git worktree is created alongside the workspace, see\n"
            "`jj workspace forget --help for details`.\n",
        )
        self.assertEqual(changelog.check(), [])

    def test_breaking(self):
        path = self.new(["3", "The MSRV is now 1.97.1.", ""], name="msrv")
        self.assertEqual(path, self.notes_dir / "breaking" / "msrv.md")
        self.assertEqual(path.read_text(), "The MSRV is now 1.97.1.\n")
        self.assertEqual(changelog.check(), [])

    def test_reprompts_for_invalid_and_missing_answers(self):
        path = self.new(["0", "x", "4", "", "Deprecated `foo`.", ""])
        self.assertEqual(changelog.parse_note(path).type, "deprecation")

    def test_eof_on_required_text_aborts(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "aborted"):
            self.new(["1"])
        self.assertEqual(changelog.note_paths(), [])

    def test_name_collision_gets_suffix(self):
        first = self.new(["1", "Add foo.", ""])
        second = self.new(["1", "Add foo.", ""])
        third = self.new(["2", "Add foo.", ""])
        self.assertEqual(
            (first.name, second.name, third.relative_to(self.notes_dir).as_posix()),
            ("add-foo.md", "add-foo-2.md", "fix/add-foo.md"),
        )

    def test_invalid_name(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "lowercase"):
            self.new([], name="Foo Bar")


class ParseNoteTest(ChangelogTestCase):
    def assert_invalid(self, name, text, message):
        path = self.write_note(name, text)
        with self.assertRaisesRegex(changelog.ChangelogError, message):
            changelog.parse_note(path)

    def test_type_is_directory(self):
        path = self.write_note("security/note.md", "Fixed a CVE.\n")
        note = changelog.parse_note(path)
        self.assertEqual((note.type, note.body), ("security", "Fixed a CVE."))

    def test_invalid_notes(self):
        self.assert_invalid("note.md", "Body.\n", "must be in one of these directories")
        self.assert_invalid("bugfix/note.md", "Body.\n", "must be in one of these directories")
        self.assert_invalid("fix/sub/note.md", "Body.\n", "must be in one of these directories")
        self.assert_invalid("fix/note.txt", "Body.\n", "must be `.md` files")
        self.assert_invalid("fix/note.md", "\n\n", "is empty")
        self.assert_invalid("fix/note.md", "* Body.\n", "must not start with a list bullet")


class CheckTest(ChangelogTestCase):
    def test_reports_unformatted_note(self):
        self.write_note("fix/a.md", "Fixed   it.\n")
        [error] = changelog.check()
        self.assertIn("a.md: changelog note is not formatted", error)
        self.assertIn("    Fixed it.", error)

    def test_reports_misplaced_notes(self):
        self.write_note("fix/a.txt", "Fixed it.\n")
        self.write_note("b.md", "Fixed it.\n")
        self.write_note("bugfix/c.md", "Fixed it.\n")
        errors = changelog.check()
        self.assertEqual(len(errors), 3)
        for expected in (
            "a.txt: changelog notes must be `.md` files",
            "b.md: changelog notes must be in one of these directories",
            "c.md: changelog notes must be in one of these directories",
        ):
            self.assertTrue(any(expected in error for error in errors), errors)

    def test_readme_is_ignored(self):
        self.write_note("README.md", "# Notes\n")
        self.assertEqual(changelog.check(), [])

    def test_tool_files_are_ignored(self):
        for name in ("__init__.py", "changelog.py", "test_changelog.py"):
            (self.notes_dir / name).write_text("")
        (self.notes_dir / "__pycache__").mkdir()
        self.assertEqual(changelog.check(), [])
        self.assertEqual(changelog.note_paths(), [])

    def test_rejects_unreleased_heading(self):
        self.changelog.write_text(
            CHANGELOG_TEXT.replace("## [0.2.0]", "## [Unreleased]\n\n* New.\n\n## [0.2.0]")
        )
        [error] = changelog.check()
        self.assertIn("CHANGELOG.md:9: unexpected heading '## [Unreleased]'", error)

    def test_requires_one_unreleased_note(self):
        for text in (
            CHANGELOG_TEXT.replace(UNRELEASED_NOTE, ""),
            CHANGELOG_TEXT.replace(UNRELEASED_NOTE, UNRELEASED_NOTE * 2),
            CHANGELOG_TEXT.replace(
                UNRELEASED_NOTE,
                "<!-- END UNRELEASED NOTE -->\nNote.\n<!-- BEGIN UNRELEASED NOTE -->\n\n",
            ),
        ):
            with self.subTest(text=text):
                self.changelog.write_text(text)
                [error] = changelog.check()
                self.assertIn("expected one `<!-- BEGIN UNRELEASED NOTE -->` line", error)


class ReleaseTest(ChangelogTestCase):
    def test_release(self):
        self.write_note(
            "feature/b-feature.md",
            """\
            Added `jj foo --bar`, which wraps
            onto a second line.
            """,
        )
        self.write_note(
            "breaking/b-foo-breaking.md",
            "`jj foo` now requires `--bar`.\n\nSecond paragraph.\n",
        )
        self.write_note("breaking/a-msrv.md", "The MSRV is now 2.0.\n")
        self.write_note("fix/c-fix.md", "Fixed `jj foo`.\n")
        self.write_note("README.md", "# Notes\n")

        deleted = changelog.release("0.3.0", "2026-03-01")

        self.assertEqual(
            sorted(p.name for p in deleted),
            ["a-msrv.md", "b-feature.md", "b-foo-breaking.md", "c-fix.md"],
        )
        # The type directories are kept, with only their `.gitkeep` files.
        self.assertEqual(
            sorted(p.relative_to(self.notes_dir).as_posix() for p in self.notes_dir.rglob("*")),
            ["README.md"]
            + sorted(
                path
                for note_type in changelog.TYPE_SECTIONS
                for path in (note_type, f"{note_type}/.gitkeep")
            ),
        )
        self.assertEqual(
            self.changelog.read_text(),
            CHANGELOG_TEXT.replace(
                "## [0.2.0]",
                textwrap.dedent(
                    """\
                    ## [0.3.0] - 2026-03-01

                    ### Breaking changes

                    * The MSRV is now 2.0.

                    * `jj foo` now requires `--bar`.

                      Second paragraph.

                    ### New features

                    * Added `jj foo --bar`, which wraps
                      onto a second line.

                    ### Fixed bugs

                    * Fixed `jj foo`.

                    ## [0.2.0]"""
                ),
            ).replace(
                "[0.2.0]: ",
                "[0.3.0]: https://github.com/jj-vcs/jj/compare/v0.2.0...v0.3.0\n[0.2.0]: ",
            ),
        )
        self.assertEqual(changelog.check(), [])

    def test_invalid_note_aborts_without_changes(self):
        self.write_note("fix/a.md", "Fixed a.\n")
        self.write_note("nope/b.md", "Fixed b.\n")
        with self.assertRaisesRegex(changelog.ChangelogError, "b.md"):
            changelog.release("0.3.0", "2026-03-01")
        self.assertEqual(self.changelog.read_text(), CHANGELOG_TEXT)
        self.assertTrue((self.notes_dir / "fix" / "a.md").exists())

    def test_errors(self):
        self.write_note("fix/a.md", "Fixed a.\n")
        for version, changelog_text, message in (
            ("0.3", CHANGELOG_TEXT, "must look like X.Y.Z"),
            ("0.2.0", CHANGELOG_TEXT, "already exists"),
            ("0.3.0", "# Changelog\n", "no `## \\[X.Y.Z\\]"),
            ("0.3.0", CHANGELOG_TEXT.split("[0.2.0]: ")[0], "no `\\[X.Y.Z\\]: <url>`"),
        ):
            with self.subTest(message=message):
                self.changelog.write_text(changelog_text)
                with self.assertRaisesRegex(changelog.ChangelogError, message):
                    changelog.release(version, "2026-03-01")
                self.assertTrue((self.notes_dir / "fix" / "a.md").exists())

    def test_no_notes(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "no changelog notes"):
            changelog.release("0.3.0", "2026-03-01")

    def test_unreleased(self):
        self.write_note("fix/a.md", "Fixed a.\n")
        changelog.release("unreleased", None)
        self.assertEqual(
            self.changelog.read_text(),
            CHANGELOG_TEXT.replace(
                "## [0.2.0]", "## [Unreleased]\n\n### Fixed bugs\n\n* Fixed a.\n\n## [0.2.0]"
            ).replace(
                "[0.2.0]: ",
                "[Unreleased]: https://github.com/jj-vcs/jj/compare/v0.2.0...HEAD\n[0.2.0]: ",
            ),
        )
        with self.assertRaisesRegex(changelog.ChangelogError, "Unreleased already exists"):
            changelog.release("Unreleased", None)

    def test_unreleased_without_notes(self):
        changelog.release("Unreleased", None)
        self.assertIn("\n## [Unreleased]\n\n## [0.2.0]", self.changelog.read_text())

    def test_unreleased_rejects_date(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "date can't be given"):
            changelog.release("Unreleased", "2026-03-01")
        self.assertEqual(self.changelog.read_text(), CHANGELOG_TEXT)


class RemoveUnreleasedNoteTest(ChangelogTestCase):
    def test_removes_note(self):
        changelog.remove_unreleased_note_from_changelog()
        self.assertEqual(
            self.changelog.read_text(), CHANGELOG_TEXT.replace(UNRELEASED_NOTE, "")
        )

    def test_after_unreleased_release(self):
        self.write_note("fix/a.md", "Fixed a.\n")
        changelog.release("Unreleased", None)
        changelog.remove_unreleased_note_from_changelog()
        self.assertIn(
            "Intro text.\n\n## [Unreleased]\n\n### Fixed bugs\n\n* Fixed a.\n\n## [0.2.0]",
            self.changelog.read_text(),
        )

    def test_missing_markers(self):
        without_note = CHANGELOG_TEXT.replace(UNRELEASED_NOTE, "")
        self.changelog.write_text(without_note)
        with self.assertRaisesRegex(changelog.ChangelogError, "expected one"):
            changelog.remove_unreleased_note_from_changelog()
        self.assertEqual(self.changelog.read_text(), without_note)


if __name__ == "__main__":
    unittest.main()
