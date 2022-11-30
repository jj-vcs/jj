# Jujutsu VCS

![](https://img.shields.io/github/license/martinvonz/jj) ![](https://img.shields.io/github/v/release/martinvonz/jj) ![](https://img.shields.io/github/release-date/martinvonz/jj) ![](https://img.shields.io/crates/v/jujutsu)
<br/>
![](https://github.com/martinvonz/jj/workflows/build/badge.svg) ![](https://img.shields.io/codefactor/grade/github/martinvonz/jj/main) ![](https://img.shields.io/librariesio/github/martinvonz/jj)

- [Disclaimer](#disclaimer)
- [Introduction](#introduction)
- [Status](#status)
- [Installation](#installation)
- [Command-line completion](#command-line-completion)
- [Getting started](#getting-started)
- [Related work](#related-work)

## Disclaimer

This is not a Google product. It is an experimental version-control system
(VCS). It was written by me, Martin von Zweigbergk (martinvonz@google.com). It
is my personal hobby project and my 20% project at Google. It does not indicate
any commitment or direction from Google.


## Introduction

Jujutsu is a [Git-compatible](docs/git-compatibility.md)
[DVCS](https://en.wikipedia.org/wiki/Distributed_version_control). It combines
features from Git (data model,
[speed](https://github.com/martinvonz/jj/discussions/49)), Mercurial (anonymous
branching, simple CLI [free from "the index"](docs/git-comparison.md#the-index),
[revsets](docs/revsets.md), powerful history-rewriting), and Pijul/Darcs
([first-class conflicts](docs/conflicts.md)), with features not found in most
of them ([working-copy-as-a-commit](docs/working-copy.md),
[undo functionality](docs/operation-log.md), automatic rebase,
[safe replication via `rsync`, Dropbox, or distributed file
system](docs/technical/concurrency.md)).

The command-line tool is called `jj` for now because it's easy to type and easy
to replace (rare in English). The project is called "Jujutsu" because it matches
"jj".

If you have any questions, please join us on Discord
[![Discord](https://img.shields.io/discord/968932220549103686.svg?label=&logo=discord&logoColor=ffffff&color=7389D8&labelColor=6A7EC2)](https://discord.gg/dkmfj3aGQN).

## Features

### Compatible with Git

Jujutsu has two backends. One of them is a Git backend (the other is a native
one [^native-backend]). This lets you use Jujutsu as an alternative interface to Git. The commits
you create will look like regular Git commits. You can always switch back to
Git. The Git support uses the [libgit2](https://libgit2.org/) C library.

[^native-backend]: At this time, there's practically no reason to use the native
backend (the only minor reason might be
[#27](https://github.com/martinvonz/jj/issues/27)).
The backend exists mainly to make sure that it's possible to eventually add
functionality that cannot easily be added to the Git backend.

<img src="demos/git_compat.png" />

### The working copy is automatically committed

Most Jujutsu commands automatically commit the working copy. This leads to a
simpler and more powerful interface, since all commands work the same way on the
working copy or any other commit. It also means that you can always check out a
different commit without first explicitly committing the working copy changes
(you can even check out a different commit while resolving merge conflicts).

<img src="demos/working_copy.png" />

### Operations update the repo first, then possibly the working copy

The working copy is only updated at the end of an operation, after all other
changes have already been recorded. This means that you can run any command
(such as `jj rebase`) even if the working copy is dirty.

### Entire repo is under version control

All operations you perform in the repo are recorded, along with a snapshot of
the repo state after the operation. This means that you can easily revert to an
earlier repo state, or to simply undo a particular operation (which does not
necessarily have to be the most recent operation).

<img src="demos/operation_log.png" />

### Conflicts can be recorded in commits

If an operation results in conflicts, information about those conflicts will be
recorded in the commit(s). The operation will succeed. You can then resolve the
conflicts later. One consequence of this design is that there's no need to
continue interrupted operations. Instead, you get a single workflow for
resolving conflicts, regardless of which command caused them. This design also
lets Jujutsu rebase merge commits correctly (unlike both Git and Mercurial).

Basic conflict resolution:

<img src="demos/resolve_conflicts.png" />

Juggling conflicts:

<img src="demos/juggle_conflicts.png" />

### Automatic rebase

Whenever you modify a commit, any descendants of the old commit will be rebased
onto the new commit. Thanks to the conflict design described above, that can be
done even if there are conflicts. Branches pointing to rebased commits will be
updated. So will the working copy if it points to a rebased commit.

### Comprehensive support for rewriting history

Besides the usual rebase command, there's `jj describe` for editing the
description (commit message) of an arbitrary commit. There's also `jj touchup`,
which lets you edit the changes in a commit without checking it out. To split
a commit into two, use `jj split`. You can even move part of the changes in a
commit to any other commit using `jj move`.


## Status

The tool is quite feature-complete, but some important features like (the
equivalent of) `git blame` are not yet supported. There
are also several performance bugs. It's also likely that workflows and setups
different from what the core developers use are not well supported.

I (Martin von Zweigbergk) have almost exclusively used `jj` to develop the
project itself since early January 2021. I haven't had to re-clone from source
(I don't think I've even had to restore from backup).

There *will* be changes to workflows and backward-incompatible changes to the
on-disk formats before version 1.0.0. Even the binary's name may change (i.e.
away from `jj`). For any format changes, we'll try to implement transparent
upgrades (as we've done with recent changes), or provide upgrade commands or
scripts if requested.


## Installation

See below for how to build from source. There are also
[pre-built binaries](https://github.com/martinvonz/jj/releases) for Windows,
Mac, or Linux (musl).

If you're installing from source, you need to use Rust version 1.60 or higher,
or you will get a cryptic message like this:
```
error: failed to select a version for the requirement `libgit2-sys = "=0.14.0"``
candidate versions found which didn't match: 0.13.2+1.4.2, 0.13.1+1.4.2, 0.13.0+1.4.1, ...
```

### Linux

On most distributions, you'll need to build from source using `cargo` directly.

#### Build using `cargo`

First make sure that you have the `libssl-dev` and `openssl` packages installed
by running something like this:
```shell script
sudo apt-get install libssl-dev openssl
```

Now run:
```shell script
cargo install --git https://github.com/martinvonz/jj.git --bin jj
```


#### Nix OS

If you're on Nix OS you can use the flake for this repository.
For example, if you want to run `jj` loaded from the flake, use:

```shell script
nix run 'github:martinvonz/jj'
```

You can also add this flake url to your system input flakes. Or you can
install the flake to your user profile:

```shell script
nix profile install 'github:martinvonz/jj'
```


#### Homebrew

If you use linuxbrew, you can run:

```shell script
brew install jj
```


### Mac

#### Homebrew

If you use Homebrew, you can run:

```shell script
brew install jj
```


#### From Source

You may need to run some or all of these:
```shell script
xcode-select --install
brew install openssl
brew install pkg-config
export PKG_CONFIG_PATH="$(brew --prefix)/opt/openssl@3/lib/pkgconfig"
```

Now run:
```shell script
cargo install --git https://github.com/martinvonz/jj.git --bin jj
```


### Windows

Run:
```shell script
cargo install --git https://github.com/martinvonz/jj.git --bin jj
```


## Initial configuration

You may want to configure your name and email so commits are made in your name.
Create a file at `~/.jjconfig.toml` and make it look something like
this:
```shell script
$ cat ~/.jjconfig.toml
[user]
name = "Martin von Zweigbergk"
email = "martinvonz@google.com"
```


## Command-line completion

To set up command-line completion, source the output of
`jj debug completion --bash/--zsh/--fish`. Exactly how to source it depends on
your shell.

### Bash
```shell script
source <(jj debug completion)  # --bash is the default
```

### Zsh
```shell script
autoload -U compinit
compinit
source <(jj debug completion --zsh | sed '$d')  # remove the last line
compdef _jj jj
```

### Fish
```shell script
jj debug completion --fish | source
```

### Xonsh
```shell script
source-bash $(jj debug completion)
```


## Getting started

The best way to get started is probably to go through
[the tutorial](docs/tutorial.md). Also see the
[Git comparison](docs/git-comparison.md), which includes a table of
`jj` vs. `git` commands.


## Related work

There are several tools trying to solve similar problems as Jujutsu. See
[related work](docs/related-work.md) for details.
