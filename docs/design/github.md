# GitHub

This is an aspirational document that describes how jj _will_ support 
GitHub as a forge. Readers are assumed to have some familiarity with Git
and GitHub.

Some sections of this document are work in progress and there may still be
sections missing for providing better integration between GitHub and local
state in jj.

## Objective

The goal of this integration is to provide some management of GitHub's forge
features with jj.
I.e. a user should be able to handle the most common commands via `jj` and
introspect the state in `jj` as well.

The code content exchange is done via `jj git`. So the main features are
pull request related:
* Create a pull request
* Merge a pull request

### Non-Goals

* This document does not cover equivalent commands on other forges.
* Replace `jj git` commands when working with GitHub

## Background

### Pull Request Data

Pull requests have various bits of data associated with them.
Generally they have a `head` branch (the code that should be added) and a
`base` branch (the branch the code should be added to).
While the `base` branch can be changed, the `head` is constant.
On the other hand, both the `base` and `head` references can be updated to
point to other commits.

On top of the branches, pull requests also contain data about:
* A title
* A summary
* Mergeability

and many more. But most other data (e.g. review comments) do not have a good
equivalent form in `jj`'s data model.

The `head` branch may or may not be automatically deleted by GitHub after a
branch is merged.

### Modes of Operation for Pull Requests

GitHub generally has two modes we need to consider.
One is more common for OSS while the other is generally used for commercial
products.

#### Fork & Cross Repository Pull Request

The common OSS way of operation is for the user to fork the main repository
into their own repository namespace.
From there they create pull requests towards the main repository.
I.e. a pull request may be created from `Ongy/jj` into `jj-vcs/jj`.
The repositories largely share git objects, but have independent refs
(i.e. branches and tags) and permissions.

#### Branch to main

The common use case for proprietary projects is a single (private) repository
where contributors have the ability to push, but some branches may be protected.
In this case, pull requests are done within a single repository.
I.e. it would be from `jj-vcs/jj` into `jj-vcs/jj`.

### Type of Merges

GitHub provides different methods of merging pull requests.
These will lead to different resulting git commit states.
Some of the resulting states will map well onto `jj`'s view of visible changes.
Others require some amount of management to achieve a desirable collection of
changes locally.

#### Merge

When the suggested branch is merged with a merge commit, `jj` will detect the
contained changes to now be ancestors of remote branches and hide them by
default.

No additional management needs to be done.

#### Rebase

When the suggested branch is rebased onto the target branch, the commit
structure persists, but the commits that are now ancestors of a remote branch
are new commits created during the merge.

Additional management is required.

#### Squash

When the suggested branch is squashed into a single commit, there is no longer
any relation between the commits previously on the `head` branch and the commit
added to the `base` branch.

Additional management is required.

## Pull Requests

While GitHub allows to have multiple pull requests associated with a branch, we
will assume that every remote bookmark has at most one canonical pull request.

The `head` of the pull request will be the branch associated with the remote
bookmark.
The `base` will be the closest parent with a remote bookmark.
When there is more than one candidate `base`, operations that require a base
to be determined will detect this deviation from the model and provide the
user with an explanation and exit with an error.

### Creation

A command to create pull requests will be provided. Provisionally
`jj github pull-request create`.
It will operate on a revset, defaulting to `immutable()..@`.
It will create a stack of pull requests for remote tracked bookmarks in the
revset.

To provide a summary and body for the pull request, the editor will be opened.
It will be pre-populated with the description of changes in the pull request.

This will handle both multiple pull requests, and multiple changes per PR.

#### Cross Repository PR Chain

There's still some open questions about whether we can properly do a PR stack
on cross-repository setups.
This configuration might not support a stack.

### Submitting

A command to merge fully reviewed and ready pull requessts will be provided.
Provisionally `jj github pull-request merge`. 
By default it will warn, but not prevent, merging into non-upstream branches.

If there's more than one merge strategy available, it will ask the user which
method should be chosen.
There is no automatic default, but a merge strategy will be configurable and
taken as command line argument.

If the merge strategy requires creating a commit (merge, squash) the suggested
title and body will be opened in an editor for user review.

#### Merge Queues

Help requested.

### Status

A command to introspect current pull request state will be provided.
Provisionally `jj github pull-request status`. 

It'll inform the user whether a PR is ready to be merged.
In future it should provide a user with a reasonable guess whether they are
required to make changes to the PR, or are currently waiting for input.