# Coming from Git

> This tutorial is about guiding someone who has a working Git repository
> to get started with Jujutsu quickly. It assumes the reader is at least vaguely
> familiar with the basics. Get started with understanding the basic concepts at
> our [tutorial](../tutorial.md).

## Minimal risk

Since Jujutsu is compatible with Git, you can colocate a Jujutsu repository
and an existing Git repository. This makes adopting Jujutsu very easy as well as
minimally risky: if something breaks or you want to go back, just delete your
project's `.jj/` directory.

## jj init for existing Git repositories

Assuming you have [installed](../install-and-setup.md) Jujutsu, you can
initialize a
[colocated repository](../git-compatibility/#colocated-jujutsugit-repos)
in an existing Git repository on your computer like so:

```sh
cd ~/whatever/project
jj git init
```

You can even do this in a Git repository with a dirty working directory.

## Write the code

Write some code or decide it's time for a commit for some of your current
changes in the working directory.

Write the commit message:

```sh
jj describe
```

Awesome! On most `jj` commands, the first thing Jujutsu will do is auto-commit
all changes (with an empty commit message), so what you just did with
`jj describe` is to  write this _existing_ commit's message. This commit
includes all the current changes.

## Splitting instead of committing

You wrote a bunch of code but some of it can't go in that single commit. What
you do now is:

```sh
jj split
```

Now, if you have a terminal without a mouse (or have mouse completely disabled
in your terminal because it constantly gets in the way, like I do) this next
screen might confuse you. Interestingly, it was the reason I gave up on jj the
first time. Here's how this "interactive" screen works:

1. Use the arrows to go up and down the file list
1. Hit space to select the files that will go to the first commit
1. Press `c` to confirm your selection
1. Change the commit message for that first commit, and save
1. Change the commit message for that second commit, and save

Of course, for that second commit, you might decide to split again later. That's
part of the game.

## Time to push

Ok, work done, how to push?

We don't push branches in Jujutsu as we do in Git. Instead, we push `bookmarks`.
Branches are like tree branches: they fork off `main` and grow. Bookmarks are
like book _marks_: you constantly grow the book and at some points you put
bookmarks. This means that in Git every new commit is automatically part of a
growing branch. In Jujutsu, every new commit is part of the whole book and
nothing else. If you want it to be part of a Git branch, you assign a bookmark
to it:

```sh
jj bookmark create fix-login -r @
# this means I create a new bookmark in the latest/current commit (signified by @)

jj git push -b fix-login --allow-new
# this means push bookmark/branch to default origin and also create the branch
```

## Two small tips

You can always do:

```sh
jj status
# or jj st
```

to see an overview of the working directory.

You can always do:

```sh
jj log
```

to see the most recent relevant commits (i.e. working copy commit and ancestor
plus any other mutable — unpushed — commits) along with messages and any
bookmarks on those commits.

## More changes

You pushed your bookmark branch, opened a PR on GitHub, asked for review, and
some annoying nitpickers ask for changes. Fine:

```sh
vim code.txt
# :wq
jj describe -m "fix serious issue"

jj bookmark move -f fix-login -t @
# this means move the fix-login bookmark from [-f] its original place to [-t] latest commit (aka @)

jj git push -b fix-login
# and then just push
```

## Concluding the workflow

After pushing the changes, you get an approval, and merge your PR. Now what?
Pull changes:

```sh
jj git fetch
# git fetch, classic

jj new main
# prepare new commit following the latest commit of main branch, not fix-login

# or:
jj new
# prepare new commit following the last one, part of fix-login branch
```

...and that's the end of the standard GitHub workflow.
