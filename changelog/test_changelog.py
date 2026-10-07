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

# The real implementation, which the tests otherwise replace to avoid network
# access.
fetch_compare_commits = changelog.fetch_compare_commits

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
        self.notes_dir.mkdir()
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
        path.write_text(textwrap.dedent(text))
        return path

    def new(self, answers, name=None):
        return changelog.new_note(name, scripted_input(answers), lambda _: None)


class NewNoteTest(ChangelogTestCase):
    def test_feature_is_wrapped_and_normalized(self):
        path = self.new(
            [
                "1",
                "",
                ("* `jj workspace add` supports   `--colocate`/`--no-colocate` flags to "
                "control whether a Git worktree is created alongside the workspace,"),
                "see `jj workspace forget --help for details`.",
                "",
            ]
        )
        self.assertEqual(path.name, "jj-workspace-add-supports-colocate-no.md")
        self.assertEqual(
            path.read_text(),
            "---\n"
            "type: feature\n"
            "---\n"
            "`jj workspace add` supports `--colocate`/`--no-colocate` flags to control\n"
            "whether a Git worktree is created alongside the workspace, see\n"
            "`jj workspace forget --help for details`.\n",
        )
        self.assertEqual(changelog.check(), [])

    def test_breaking_note_round_trips_through_yaml(self):
        breaking = (
            "`jj split` now opens a single editor: the descriptions of all split "
            "commits are edited together, which # might surprise scripts that "
            "relied on the old behavior."
        )
        path = self.new(["2", breaking, "", "Fixed `jj split` editing.", ""])
        note = changelog.parse_note(path)
        self.assertEqual(note.type, "fix")
        self.assertEqual(note.body, "Fixed `jj split` editing.")
        self.assertEqual(note.breaking_note, changelog.format_markdown(breaking))
        self.assertIn("\n", note.breaking_note)
        self.assertEqual(changelog.check(), [])

    def test_breaking_only(self):
        path = self.new(["6", "The MSRV is now 1.97.1.", ""], name="msrv")
        self.assertEqual(path.name, "msrv.md")
        self.assertEqual(
            path.read_text(),
            "---\nbreaking_note: |\n  The MSRV is now 1.97.1.\n---\n",
        )
        note = changelog.parse_note(path)
        self.assertEqual((note.type, note.body), (None, None))

    def test_reprompts_for_invalid_and_missing_answers(self):
        path = self.new(["0", "x", "3", "", "", "Deprecated `foo`.", ""])
        self.assertEqual(changelog.parse_note(path).type, "deprecation")

    def test_eof_on_required_text_aborts(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "aborted"):
            self.new(["1", ""])
        self.assertEqual(list(self.notes_dir.iterdir()), [])

    def test_name_collision_gets_suffix(self):
        first = self.new(["1", "", "Add foo.", ""])
        second = self.new(["1", "", "Add foo.", ""])
        self.assertEqual((first.name, second.name), ("add-foo.md", "add-foo-2.md"))

    def test_invalid_name(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "lowercase"):
            self.new([], name="Foo Bar")


class ParseNoteTest(ChangelogTestCase):
    def assert_invalid(self, text, message):
        path = self.write_note("note.md", text)
        with self.assertRaisesRegex(changelog.ChangelogError, message):
            changelog.parse_note(path)

    def test_invalid_notes(self):
        self.assert_invalid("Body only.\n", "must start with")
        self.assert_invalid("---\ntype: feature\nBody.\n", "not closed")
        self.assert_invalid("---\ntype: [\n---\nBody.\n", "invalid YAML")
        self.assert_invalid("---\n- a\n---\nBody.\n", "must be a YAML mapping")
        self.assert_invalid("---\nbreaking: true\n---\nBody.\n", "unknown front matter keys: breaking")
        self.assert_invalid("---\ntype: bugfix\n---\nBody.\n", "must be one of")
        self.assert_invalid("---\ntype: fix\nbreaking_note: true\n---\nBody.\n", "must be a string")
        self.assert_invalid("---\ntype: fix\n---\n\n", "requires an entry body")
        self.assert_invalid("---\n---\nBody.\n", "requires a `type`")
        self.assert_invalid("---\n---\n", "needs a `type`")
        self.assert_invalid("---\ntype: fix\n---\n* Body.\n", "must not start with a list bullet")


class CheckTest(ChangelogTestCase):
    def test_reports_unformatted_note(self):
        self.write_note("a.md", "---\ntype: fix\n---\nFixed   it.\n")
        [error] = changelog.check()
        self.assertIn("a.md: entry body is not formatted", error)
        self.assertIn("    Fixed it.", error)

    def test_reports_non_markdown_file(self):
        self.write_note("a.txt", "---\ntype: fix\n---\nFixed it.\n")
        [error] = changelog.check()
        self.assertIn("must have a `.md` extension", error)

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
            "b-feature.md",
            """\
            ---
            type: feature
            breaking_note: |
              `jj foo` now requires `--bar`.

              Second paragraph.
            ---
            Added `jj foo --bar`, which wraps
            onto a second line.
            """,
        )
        self.write_note("a-msrv.md", "---\nbreaking_note: The MSRV is now 2.0.\n---\n")
        self.write_note("c-fix.md", "---\ntype: fix\n---\nFixed `jj foo`.\n")
        self.write_note("README.md", "# Notes\n")

        released = changelog.release("0.3.0", "2026-03-01")

        self.assertEqual(
            sorted(p.name for p in released.notes), ["a-msrv.md", "b-feature.md", "c-fix.md"]
        )
        # The fake GitHub API returned no commits.
        self.assertFalse(released.contributors_listed)
        self.assertEqual(sorted(p.name for p in self.notes_dir.iterdir()), ["README.md"])
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
        self.write_note("a.md", "---\ntype: fix\n---\nFixed a.\n")
        self.write_note("b.md", "---\ntype: nope\n---\nFixed b.\n")
        with self.assertRaisesRegex(changelog.ChangelogError, "b.md"):
            changelog.release("0.3.0", "2026-03-01")
        self.assertEqual(self.changelog.read_text(), CHANGELOG_TEXT)
        self.assertEqual(len(list(self.notes_dir.iterdir())), 2)

    def test_errors(self):
        self.write_note("a.md", "---\ntype: fix\n---\nFixed a.\n")
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
                self.assertTrue((self.notes_dir / "a.md").exists())

    def test_no_notes(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "no changelog notes"):
            changelog.release("0.3.0", "2026-03-01")

    def test_unreleased(self):
        self.write_note("a.md", "---\ntype: fix\n---\nFixed a.\n")
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
        self.write_note("a.md", "---\ntype: fix\n---\nFixed a.\n")
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
        self.write_note("a.md", "---\ntype: fix\n---\nFixed a.\n")
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
        self.write_note("a.md", "---\ntype: fix\n---\nFixed a.\n")
        released = changelog.release("0.3.0", "2026-03-01", contributors=False)
        self.fetch.assert_not_called()
        self.assertFalse(released.contributors_listed)
        self.assertNotIn("### Contributors", self.changelog.read_text())

    def test_fetch_failure_changes_nothing(self):
        self.write_note("a.md", "---\ntype: fix\n---\nFixed a.\n")
        self.fetch.side_effect = changelog.ChangelogError("gh failed")
        with self.assertRaisesRegex(changelog.ChangelogError, "gh failed"):
            changelog.release("0.3.0", "2026-03-01")
        self.assertEqual(self.changelog.read_text(), CHANGELOG_TEXT)
        self.assertTrue((self.notes_dir / "a.md").exists())

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
            commits = fetch_compare_commits("v0.1.0", "main")
        self.assertEqual(
            run.call_args.args[0],
            ["gh", "api", "/repos/jj-vcs/jj/compare/v0.1.0...main", "--paginate"],
        )
        return commits

    def test_concatenated_pages(self):
        stdout = '{"commits": [{"sha": "1"}], "files": []}\n{"commits": [{"sha": "2"}]}\n'
        commits = self.run_gh(returncode=0, stdout=stdout, stderr="")
        self.assertEqual(commits, [{"sha": "1"}, {"sha": "2"}])

    def test_gh_failure(self):
        with self.assertRaisesRegex(changelog.ChangelogError, "HTTP 404.*\n.*--no-contributors"):
            self.run_gh(returncode=1, stdout="", stderr="gh: Not Found (HTTP 404)\n")

    def test_gh_missing(self):
        with mock.patch.object(subprocess, "run", side_effect=FileNotFoundError):
            with self.assertRaisesRegex(changelog.ChangelogError, "requires the GitHub CLI"):
                fetch_compare_commits("v0.1.0", "main")


if __name__ == "__main__":
    unittest.main()
