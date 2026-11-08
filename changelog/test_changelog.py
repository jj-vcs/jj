"""Tests for the changelog tool in changelog.py.

Run with `uv run python changelog/test_changelog.py`.
"""

from __future__ import annotations

import subprocess
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


def commit(name, email, login):
    """Returns a commit as listed by GitHub's compare API."""
    return {
        "commit": {"author": {"name": name, "email": email}},
        "author": {"login": login} if login is not None else None,
    }


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
        # GitHub's compare API, which `release` calls to list contributors.
        self.fetch = mock.Mock(return_value=[])
        for name, value in (
            ("REPO_ROOT", self.root),
            ("NOTES_DIR", self.notes_dir),
            ("CHANGELOG", self.changelog),
            ("fetch_compare_commits", self.fetch),
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
                ("* `jj workspace add` supports   `--colocate`/`--no-colocate` flags to "
                "control whether a Git worktree is created alongside the workspace,"),
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

        released = changelog.release("0.3.0", "2026-03-01")

        self.assertEqual(
            sorted(p.name for p in released.notes),
            ["a-msrv.md", "b-feature.md", "b-foo-breaking.md", "c-fix.md"],
        )
        # The fake GitHub API returned no commits.
        self.assertFalse(released.contributors_listed)
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

    def test_unreleased_does_not_list_contributors(self):
        changelog.release("Unreleased", None)
        self.fetch.assert_not_called()
        with self.assertRaisesRegex(changelog.ChangelogError, "contributors can't be listed"):
            changelog.release("Unreleased", None, contributors=True)


class ContributorsTest(ChangelogTestCase):
    def test_release_lists_contributors(self):
        self.write_note("fix/a.md", "Fixed a.\n")
        self.fetch.return_value = [commit("Bob", "bob@example.com", "bob")]
        released = changelog.release("0.3.0", "2026-03-01")
        self.fetch.assert_called_once_with("v0.2.0", "main")
        self.assertTrue(released.contributors_listed)
        self.assertIn(
            textwrap.dedent(
                """\
                ## [0.3.0] - 2026-03-01

                ### Fixed bugs

                * Fixed a.

                ### Contributors

                Thanks to the people who made this release happen!

                * Bob (@bob)

                ## [0.2.0]"""
            ),
            self.changelog.read_text(),
        )

    def test_no_contributors(self):
        self.write_note("fix/a.md", "Fixed a.\n")
        released = changelog.release("0.3.0", "2026-03-01", contributors=False)
        self.fetch.assert_not_called()
        self.assertFalse(released.contributors_listed)
        self.assertNotIn("### Contributors", self.changelog.read_text())

    def test_fetch_failure_changes_nothing(self):
        self.write_note("fix/a.md", "Fixed a.\n")
        self.fetch.side_effect = changelog.ChangelogError("gh failed")
        with self.assertRaisesRegex(changelog.ChangelogError, "gh failed"):
            changelog.release("0.3.0", "2026-03-01")
        self.assertEqual(self.changelog.read_text(), CHANGELOG_TEXT)
        self.assertTrue((self.notes_dir / "fix" / "a.md").exists())

    def test_format_contributors(self):
        commits = [
            commit("Bob", "bob@example.com", "bob"),
            commit("Dependabot", "bot@example.com", "dependabot[bot]"),
            commit("alice", "alice@example.com", None),
            # Same GitHub account with a different name: the first name wins.
            commit("Bob Smith", "bob2@example.com", "bob"),
            # Same unlinked email address, compared case-insensitively.
            commit("Alice A.", "Alice@Example.com", None),
            commit("Carol", "carol@example.com", None),
            commit("zed", "zed@example.com", "Zed"),
            commit("Gasper", "gasper@example.com", "gasper"),
            commit("Gaëtan", "gaetan@example.com", "gaetan"),
        ]
        self.assertEqual(
            changelog.format_contributors(commits),
            ["alice", "Bob (@bob)", "Carol", "Gaëtan (@gaetan)", "Gasper (@gasper)", "zed (@Zed)"],
        )


class FetchCompareCommitsTest(unittest.TestCase):
    def run_gh(self, **result):
        completed = subprocess.CompletedProcess(args=[], **result)
        with mock.patch.object(subprocess, "run", return_value=completed) as run:
            commits = changelog.fetch_compare_commits("v0.1.0", "main")
        self.assertEqual(
            run.call_args.args[0],
            [
                "gh",
                "api",
                "/repos/jj-vcs/jj/compare/v0.1.0...main",
                "--paginate",
                "--jq",
                ".commits[]",
            ],
        )
        return commits

    def test_one_commit_per_line(self):
        stdout = '{"sha": "1"}\n{"sha": "2"}\n'
        commits = self.run_gh(returncode=0, stdout=stdout, stderr="")
        self.assertEqual(commits, [{"sha": "1"}, {"sha": "2"}])

    def test_gh_failure(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "HTTP 404.*\n.*--no-contributors"):
            self.run_gh(returncode=1, stdout="", stderr="gh: Not Found (HTTP 404)\n")

    def test_gh_missing(self):
        with mock.patch.object(subprocess, "run", side_effect=FileNotFoundError):
            with self.assertRaisesRegex(changelog.ChangelogError, "requires the GitHub CLI"):
                changelog.fetch_compare_commits("v0.1.0", "main")


if __name__ == "__main__":
    unittest.main()
