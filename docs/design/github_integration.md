# GitHub Integration

> Native GitHub integration for creating pull requests from bookmarks

Author: [Parth Doshi](mailto:contact@parthdoshi.com)

## Summary

This design proposes basic GitHub integration for jj that allows users to create pull requests directly from single revisions without relying on external tools for the core workflow. The implementation will shell out to the GitHub CLI (`gh`) for authentication and API access, focusing on simple revision-based PR creation with smart base detection.

## State of the Feature as of Current Version

Currently, jj users must manually switch to external tools like the GitHub CLI (`gh`) or use the GitHub web interface to create pull requests from their jj workflows. This creates friction and breaks the native jj experience, requiring users to understand the mapping between jj concepts (bookmarks, revisions) and GitHub concepts (branches, commits).

## Goals and non-goals

**Goals:**
- Create pull requests from a single revision using `jj github pr create`
- Automatically determine appropriate base branches from ancestor bookmarks
- Clear separation: `--remote` controls push location, `--base` controls PR target
- Leverage existing `gh` CLI authentication to avoid key management
- Provide native jj experience that works with jj's revision-based mental model
- Reuse existing `jj git push` bookmark creation logic for consistency

**Non-goals (for initial implementation):**
- Multiple revision support (future possibility)
- Interactive TUI for stacked PR creation (future possibility)
- Topics integration (may be implemented later when topics are available)
- Complex forge abstraction layers for multiple platforms
- Managing GitHub API keys directly within jj
- Full GitHub issue management
- GitHub Actions integration
- Advanced PR management (updating, closing, merging)

## Overview

The GitHub integration will add a new `jj github` subcommand that provides simple revision-based PR creation functionality. The implementation will evaluate the specified revision (defaulting to `@`), determine appropriate base branches by finding ancestor bookmarks that have been pushed to GitHub, resolve the GitHub repository from git remotes, and create a pull request using GitHub's API via the `gh` CLI tool.

### Detailed Design

### Authentication Strategy

The initial implementation will rely on the GitHub CLI (`gh`) being installed and authenticated. This approach:

- Avoids the complexity of managing API keys and authentication flows
- Leverages existing user setups
- Provides immediate functionality without additional configuration

The implementation will shell out to `gh auth status` to verify authentication and use `gh api` commands for GitHub API interactions.

### Command Interface

The GitHub integration adds a `jj github pr create` command that creates pull requests from revisions with smart defaults:

```bash
# Create PR from current revision with smart defaults
jj github pr create

# Create PR from specific revision
jj github pr create -r abc123

# Override the base branch (same repository)
jj github pr create --base main

# Cross-repository PR (fork workflow)
jj github pr create --base upstream:main

# Push to different remote but same repo PR
jj github pr create --remote upstream

# Skip confirmation prompts
jj github pr create --yes
```

### Implementation Details

1. **Revision Evaluation**: Parse and evaluate the provided revision (defaults to `@` if not specified)
2. **Remote Resolution**:
   - **Push remote**: Use `--remote` flag, `git.push` setting, or fall back to `origin`
   - **PR repository**: If `--base` specifies `remote:branch`, use that remote; otherwise use same as push remote
3. **Base Branch Detection**: Find the most appropriate base by:
   - If `--base` specified: parse as `[remote:]branch` and use directly
   - Otherwise: Look for closest ancestor bookmark that has corresponding remote branch
   - **Error if ancestor bookmark exists but isn't pushed** - prompt user to push it first
   - Fall back to common base branches (main/master) if no ancestor bookmark exists
4. **Bookmark Creation**: If the revision doesn't have a bookmark:
   - **Reuse `jj git push` logic**: Use existing bookmark naming from `templates.git_push_bookmark` setting
   - Create bookmark using same naming scheme as `jj git push -c @`
5. **Smart Defaults**:
   - **PR title**: First line of the commit message
   - **PR description**: PR template if present, otherwise full commit description
6. **Preview and Confirmation**: Show user exactly what will happen before any actions

#### Command Options (Implementation Detail)

**Usage:** `jj github pr create [OPTIONS]`

* `-r, --revisions <REVISIONS>` — The revision to create a PR for (default: `@`)

* `--base <BASE>` — Override the automatically detected base branch. Can be specified as `branch` (same repository) or `remote:branch` (cross-repository PR). By default, the base is determined by finding the closest ancestor bookmark that exists on a remote. If no ancestor bookmark is found, falls back to the default branch (main/master).

* `--remote <REMOTE>` — The remote to push the branch to. This only controls where the branch will be pushed. The PR repository is determined by the base branch (if specified as `remote:branch`) or defaults to the same repository as the push remote. Defaults to the `git.push` setting, or "origin" if multiple remotes exist and no push default is configured.

* `--title <TITLE>` — Override the PR title (default: first line of commit message)

* `--body <BODY>` — Override the PR description (default: PR template or commit description)

* `--draft` — Create a draft pull request

* `--dry-run` — Show what would be done without actually pushing or creating the PR

* `-y, --yes` — Skip confirmation prompts and proceed automatically

### Preview Interface

Before taking any actions, the command will show a clear preview:

```
jj github pr create

Push:
  Branch: pr-abc123 → origin

Pull Request:
  Repository: upstream/myproject
  From: origin:pr-abc123 "feat: add user authentication"
  To: upstream:main
  Title: "feat: add user authentication"
  Description: [using .github/pull_request_template.md]

Push branch? [Y/n]
Create PR? [Y/n]
```

### Error Handling Examples

**Unpushed ancestor bookmark:**
```
Error: Ancestor bookmark 'feature-base' exists but hasn't been pushed to origin.

Push it first:
  jj git push -b feature-base

Or specify a different base:
  jj github pr create --base main
```

**Additional error cases:**
- Verify `gh` CLI is installed and authenticated
- Handle invalid revisions with clear error messages
- Validate GitHub remote configuration
- Handle cases where no suitable base branch can be determined

## Alternatives considered

1. **Bookmark-only Interface**: Initially considered limiting input to bookmarks only, but this would exclude the common case of unbookmarked work.

2. **Direct GitHub API Integration**: Implementing GitHub API calls directly would require managing OAuth flows, personal access tokens, and/or API client libraries. This adds significant complexity for authentication and credential management that I don't really want to deal with.

3. **Embedded GitHub CLI**: Bundling or embedding the `gh` CLI tool would increase the dependency footprint and maintenance burden, while providing little benefit over shelling out to an existing installation.

4. **Simple Branch-based PRs**: Creating PRs that always target the main branch (like `gh` CLI) is easy enough with the existing CLI. I use `jj` for stacking changes.

## Issues addressed

- [#4555](https://github.com/jj-vcs/jj/issues/4555): Native GitHub forge integration. Though this PR only partially solves the issue since there are many additional features that could be added.

## Future Possibilities

- **Multiple Revision Support**: Allow creating multiple PRs from a revset like `main::@`
- **Interactive TUI for Stacked PRs**: Allow users to select multiple revisions and create stacked PRs with dependency management
- **Topics Integration**: When jj's topic system is implemented, integrate PR creation with topic-based workflows
- **Direct API Authentication**: Implement native GitHub API authentication to remove the `gh` CLI dependency
- **Extended GitHub Features**: Add support for issues, GitHub Actions status, PR updates and management
- **Generic Forge Support**: Expand to support GitLab, Bitbucket, and other forge platforms
- **Advanced PR Workflows**: Support for draft PRs, reviewers, labels, and other GitHub-specific features
