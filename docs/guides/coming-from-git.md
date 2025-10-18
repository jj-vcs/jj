# Coming from Git

## Minimal risk

Since `jujutsu` is compatible with `git`, you can colocate a jujutsu repository
and an existing git repository. This makes adopting jujutsu very easy as well as
minimally risky: if something breaks or you want to go back, just delete your
project's `.jj/` directory.

## jj init for existing git repositories

Assuming you have [installed](../install-and-setup.md) `jujutsu`, you can
initialize colocation in an existing git repository on your computer like so:

```sh
cd ~/whatever/project
jj git init
```

You can even do this in a git repository with a dirty working directory.

## Now write code

Write some code or decide it's time for a commit for some of your current
changes in the working directory.

Write the commit message:

```sh
jj describe
```

Awesome! `jujutsu` commits constantly by itself (with empty commit messages), so what
you just did is write the existing commit's message. This commit includes all the
current changes.

## Now split

You wrote a bunch of code but some of it can't go in that single commit. What
you do now is `jj split`. Run `jj split`:

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

`jj` does not have branches, it has `bookmarks`. Branches are like tree branches:
they fork off `main` and grow. Bookmarks are like book _marks_: you constantly
grow the book and at some points you put bookmarks. This means that in git, every new
commit is automatically part of a growing branch. In `jj`, every new commit is part
of the whole book and nothing else. If you want it to be part of a git branch,
you assign a bookmark to it:

```sh
jj bookmark set fix-login -r @
# this means I set a new bookmark in the latest/current commit (signified by @)

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
jj
# or jj log
```

to see current and previous commits along with messages and bookmarks.

## More, other, code

You pushed your bookmark branch, opened a PR on GitHub, asked for review, and
some annoying nitpickers ask for changes. Fine:

```sh
vim code.txt
# :wq
jj describe -m "fix serious issue"

jj bookmark set fix-login -r @
# this means move the fix-login bookmark to latest commit

jj git push -b fix-login
# and then just push
```

## Cigarettes after merge

After pushing the changes, you get an approval, and merge your PR. Now what?
Pull changes:

```sh
jj git fetch
# git fetch, classic

jj new main@origin
# prepare new commit following the latest commit from origin/main (written main@origin in jj)
```

...and that's the end of a standard GitHub workflow.
