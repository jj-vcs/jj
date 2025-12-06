# Multiple remotes

When using multiple [remote repositories], how you configure them in Jujutsu
depends on your workflow and the role each remote plays.

The setup varies based on whether you are contributing to an upstream project,
or integrating changes from another repository.

[remote repositories]: ../glossary.md#remote

## Nomenclature

A remote named `origin` is one you have write-access to and is usually where you
push changes.

A remote named `upstream` is the more well-known repository. You may not be able
to push to this repository.

The trunk in each repository is assumed to be `main`, so the remote bookmarks
are `main@origin` and `main@upstream`.

## Contributing upstream with a GitHub-style fork

This is a GitHub-style fork used to contribute to the upstream repository.
`upstream` is the canonical upstream remote, and `origin` is where you push
contributions, most likely so you can open pull requests.

Actions you might take:

- Fetch from `upstream` to get the latest changes.
- Push `main` to `origin` to keep it up-to-date.
- Push `my-feature` to `origin`, then open a pull request to `upstream`.

To support this scenario, you should:

- Track `main@upstream` so your local `main` branch is updated whenever you
  fetch from `upstream`.
- Track `main@origin` so when you `jj git push`, your fork's `main` branch is
  updated.
- Set `main@upstream` as the `trunk()` revset alias so it is immutable.

```shell
# Fetch from both remotes by default
$ jj config set --repo git.fetch '["upstream", "origin"]'

# Push only to the fork by default
$ jj config set --repo git.push origin

# Track both remote bookmarks
$ jj bookmark track main

# The upstream repository defines the trunk
$ jj config set --repo 'revset-aliases."trunk()"' main@upstream
```

## Maintaining an independent repository that integrates changes from upstream

This is a repository that was originally cloned from upstream, but now contains
changes in its `main` branch that are not upstream and might never be
contributed back.

- `origin` is the repository you are working in.
- `upstream` is the repository you periodically integrate changes from.

Actions you might take:

- Fetch from `origin` to get the latest changes.
- Push bookmarks to `origin`.
- Merge pull requests into `main@origin`.
- Periodically fetch from `main@upstream` and merge, rebase, or duplicate its
  changes into `main@origin`.

To support this scenario, you should:

- Track only `main@origin` so your local `main` branch is updated whenever you
  fetch from `origin`, and so you can push to it if necessary.
- _Do not_ track `main@upstream`.
- Set `main@origin` as the `trunk()` revset alias so it is immutable.

```shell
# Fetch from origin or both remotes by default
$ jj config set --repo git.fetch '["origin"]'
# or: jj config set --repo git.fetch '["upstream", "origin"]'

# Push only to origin by default
$ jj config set --repo git.push origin

# Track only the origin bookmark
$ jj bookmark track main --remote=origin
$ jj bookmark untrack main --remote=upstream

# The origin repository defines the trunk
$ jj config set --repo 'revset-aliases."trunk()"' main@origin
```

## Other workflows

Other workflows may be supported. Some general guidance for this:

- Set `trunk()` to be the remote bookmark you usually rebase upon. If you always
  rebase against upstream, set it to `main@upstream`.

- Tracking a remote bookmark `main@origin` means it and `main` represent the
  same branch. When one moves, the other should move with it. If you want them to
  _automatically_ move together, you should track the remote bookmark. If not, do
  not track it.

If you have a workflow that is not well-supported, discussion is welcome in
[Discord]. There is also an [open discussion] for enhancing how bookmark
tracking works.

[Discord]: https://discord.gg/dkmfj3aGQN
[open discussion]: https://github.com/jj-vcs/jj/issues/7072
