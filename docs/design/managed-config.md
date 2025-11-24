# Repo-managed JJ configuration

Author: [Matt Stark](mailto:msta@google.com)

Status: Pending implementation

## Background

The design doc [Secure JJ Config](secure-config.md) introduces a mechanism,
`metadata.binpb`, through which information about a repository / workspace can
be stored. It also discusses how allowing an external user to have control over
your config is a security risk.

## Overview

There is a need for a repository to impose requirements upon users. Examples of
these things include, but are not limited to:

*   Formatter configuration (eg. [Chromium Formatter Config])
*   Pre-upload checks (eg. [Chromium Pre-upload Checks])
*   Syncing scripts (eg. [Chromium Syncing Scripts])
*   Custom aliases / revsets documented by some kind of “getting started with
    <project>” document

It should be fairly obvious that there is a strong benefit to doing so. However,
controlling a user’s config is sufficient to get root access to their machine,
so we require a mechanism more complex than just blindly loading the config
file.

This is currently achieved by projects such as chromium by instructing the user
to symlink `.jj/repo/config.toml` to a file in the repository. This has several
drawbacks:

*   It doesn’t work out-of-the-box. I need to manually symlink it
*   It has no security guarantees. If I update the file in the repo, the user has no
    opportunity to review it.
*   This prevents a user from having their own repo configuration on top of it.

## Objective

*   Create a new layer of configuration between user configuration and repo
    configuration.
    *   This configuration will be stored in version control and henceforth be
        referred to as “managed” configuration.
*   Implement it in a secure manner so that an attacker cannot take control over
    the managed config.

## Detailed Design

The managed configuration will be read from `$REPO/.config/jj/config.toml`. This
is intentionally designed to be very similar to `$HOME/.config/jj/config.toml`
for the user configuration.

Any data stored here will be added to the `metadata.binpb` that was created
in [secure config](secure-config.md) (that is, we will add additional fields to
the `Metadata` struct). This will ensure that we don't suffer from the "zip file
problem".

### Security

#### Trust levels

We will add the following fields to metadata:
```
enum TrustLevel {
    // There's no "optional" in protos. It just returns the zero value.
    UNSET = 0;
    // The user wishes to completely ignore the managed config file.
    IGNORED = 1;
    // In trusted mode, we directly read from the managed config file.
    // This presents a security risk, so the user is expected to only do this
    // for a repo they trust.
    TRUSTED = 2;
    // In notify mode, the user is expected to manually replicate any changes
    // they want from the managed config to the repo config.
    NOTIFY = 3;
    // In review mode, the user has to review every config individually.
    REVIEW = 4;
}

message Metadata {
    // previous fields

    // The trust level associated with this repo.
    TrustLevel trust_level = 2;

    // When trust_level is review, the following fields are used.
    set<Hash> approved_configs = 3;
    set<Hash> rejected_configs = 4;
    // This is a can of worms and thus out of scope of the MVP.
    // There's no one correct way to choose the last approved.
    // Sometimes, the user wants be the last appproved config in the workspace.
    // At other times, the user wants be the last appproved config in the repo.
    string last_approved_config = 5;
}
```

Note that although the `Metadata` struct is stored for both the repo and the
workspace, this will only be used for the repository, not the workspace.

This is because trust is associated with repositories, not workspaces. If I
were to add an approved config in one workspace, for example, there is no
reason I wouldn't trust it in another.

#### User interface

Everything here only becomes relevant when a repo has a managed config. If it
does not have a managed config, we skip everything here.

##### Unset trust

If a repository has a managed config and unset trust, we first use
`ui.can_prompt()` to determine whether we are in a TUI.

*   If we are in a TUI, we ask the user what trust level they would like.
*   If we are not in a TUI, we warn them that we are ignoring the repo config (we
    return `IGNORE`), and that they can run `jj config managed --ignore/notify/trust` in a terminal to configure this.

##### Ignored

Nice and easy. We just completely ignore the managed config file.

##### Trusted

Nice and easy. We just read the managed config file.

##### Notify

Roughly speaking, if `mtime(repo_config_file) > mtime(managed_config)`, then
the config is up to date.

If it isn't up to date, we print a warning. The user is then expected to update
their repo config with any options they want from the managed config, which
will stop the warning from printing as the mtime has changed.

This notify option has a rather painful UX when it comes to keeping it in sync
(particularly for a user who constantly switches between different branches
synced to different versions), but I choose it for several reasons. The first is
that the vast majority of users will simply select to blindly trust the repo.
The only people who will choose this option are the very security conscious, and
this is by far the most secure mechanism. Secondly, it is so much simpler than
an approval based mechanism, as we don’t need to worry about things such as
workspaces being synced to different places. It provides far fewer edge cases.

In the future we may consider improving on the UX, doing things such as printing
a diff since the last seen config (per `last_approved_config`), but that isn't
a part of the MVP.

##### Review

This is, by far, the most difficult option. There is no doubt that this
provides value to some users, but the precise details of how this should behave
is unclear (and much harder to implement than the other options).

The biggest problem with this is that there is not a clear "correct" set of
semantics. If a config is not approved / rejected, we have 3 options:
* Fail the command. This seems like a bad idea since users have an assumption
  that most jj commands will not fail.
* Use no config. This can potentially have quite bad effects on the repo.
* Use best-effort to use an approved config (probably most recently approved).
* Force the user to make a selection before running a command
  * This is impossible in general due to `ui.can_prompt()` returning false
    * We would need to fall back to another option if we can't prompt.
  * But `ui.can_prompt()` does not mean that we *should* prompt
    * Eg. A script that runs jj to do a bisection probably doesn't want
      you to prompt every time the config changes.
    * This probably isn't too bad if it only happens once (eg. setting trust
      level), but if it happens many times, it could be very annoying.

Consider the following examples

###### User is working on config file.

1) User runs `vim .config/jj/config.toml`
2) User tries to test out the config, but needs to manually approve their own
   local change to the config every single time (this can at least be
   mitigated via setting it to trust temporarily)

###### Disabling config messes with the repo

1) User approves a config.
2) User runs `jj git fetch`. There has been an update to the managed config.
3) User runs `jj rebase -b @ -d trunk()`. The managed config is now not
   approved, and thus the next jj command will have the config disabled.

Depending on what has changed, a variety of things could happen:
* A user in a repo that defines `trunk()` could run
  `jj rebase -b <revset> -d trunk()`
  * This would result in it going to the wrong trunk
* A user in a repo that defines `immutable_heads() = ...` could run something
  like `jj rebase -b mutable() -d trunk()`
  * This would rebase a bunch of immutable commits
* A user in a repo that defines `pre-upload` hooks (which don't yet exist,
  but are being planned) runs `jj gerrit upload` and bypasses them entirely. 

Thanks to the magic of `jj undo`, everything except the upload is recoverable.
What makes them a problem, however, is the fact that a user would see the
message "please approve /reject the config" and not suspect that the command
has failed.

###### Best-effort messes with the user

If we use best-effort instead, we are likely to run into a different set of
issues.

The first is that it's no longer clear which config is being used. If I:
1) Have two workspaces A and B with the same state, both approved.
2) Sync workspace B and approve the config
3) Check out an old version of workspace A

Now which config is used?
* The most recently approved globally?
* The most recently approved for this workspace?
* Some algorithm which determines that the config for A was at a closer commit
  than B?

The second issue is that things may be incorrect. If `foo.py` was renamed to
`bar.py` and the reference was changed in `config.toml`, for example, the
config is going to break. This is not as large an issue as disabling config,
IMO, as changes will usually result in an obvious error.

###### Conclusion

The vast majority of users will not use this option, as very few users are
security-conscious, working on a repo they don't trust, and working on a repo
with managed config.

Due to:
* Significant implementation complexity
  * My prototype without this feature is only ~200 lines of code and ~100 lines
    of tests. This feature would likely double that for the most hacky simple
    implementation, and add significantly more if we wanted a proper nice UX.
* The lack of a single "correct" solution
* The fact that most users just want the "trust" option
* The fact that it is no more difficult to add other options, then add the
  review option after the fact
* The time that would be required to get consensus on the various decisions
  that would need to be made in terms of how this would work.
  * There was already a lot of
    [discussion](https://github.com/jj-vcs/jj/pull/7761#discussion_r2476135221)
    on this topic, but no clear conclusion was reached.
  * Technical (eg. disable vs best-effort)
  * UX
    * When reviewing, do we review diffs or full config files?
    * Do we prompt the user to review before running the command?

I propose that the "review" option not be put in the MVP, and and instead be
added in later.

##### Manually changing

These options will also be able to be manually set via `jj config managed
--ignore/notify/trust`.

### Where to read from

This is the trickiest part of the proposal. Consider the following workflow:

```
jj new main@origin
jj ...
jj new stable-branch@origin
```

There are some edge cases we need to consider:

*   `stable-branch` may have existed before the config was added
*   `stable-branch` may have a different copy of the managed config

The naive assumption would be that you want to read the config from `@`, as the
config will always match the version of the code you're using. However, it turns
out that some things want to refer to `@`, while others want to read the config
from `trunk()`.

Consider several different use cases:

*   My formatter was previously `clang-format --foo`, but the option `--foo` was
    deprecated in the latest version of `clang-format`
    *   Here, you want to read from `trunk()`
*   My formatter was previously `$repo/formatter --foo`, but the option `--foo`
    was deprecated in the latest version of `formatter`
    *   Here, you want to read from `@`
*   We decide to split long lines and add a new formatter (or pre-upload hook)
    config `formatter --line-length=80`
    *   Here, you probably want to read from `@`
*   We decide to add a pre-upload check that validates that all commit
    descriptions contain a reference to a bug
    *   This should be applied to old branches as well, so you want `trunk()`
*   We add a new helpful alias / revset
    *   This should be applied to old branches as well, so you want `trunk()`
*   We move our formatter
    *   If it’s external, you want to read from `trunk()`
    *   If it’s internal, you want to read from `@`

All in all, you can see a general pattern.

*   If something refers to an in-repo tool, you **probably** want the config to
    be read from `@`
*   Otherwise, you **probably** want to read from `trunk()`
*   I say probably, because the split long lines example doesn’t conform to this
    rule.

#### Problematic examples

This is problematic with `trunk()` because if you add the `--reorder-hooks` and
then checkout `stable-branch` it will incorrectly attempt to reorder imports

```
[fix.tools.rustfmt]
command = ["rustfmt", "--reorder-imports"]
```

##### Solution 1: formatter config

In practice, it is highly unlikely that a formatter config would be written that
way. Far more likely, you would see an entry in `config.toml` like:

```
[fix.tools.rustfmt]
command = ["rustfmt"]
```

`.rustfmt.toml`:
```
reorder_imports = true
```

This simply works out of the box, since the formatter is reading the config from
`@`'s `.rustfmt.toml`

##### Solution 2 (more general but convoluted): Wrapper script

As long as the formatter is in-repo, we can just write a wrapper script which
does this for us.

```
[fix.tools.rustfmt]
command = ["rustfmt.py"]
```

`rustfmt.py`:
```
os.execv(["rustfmt", "--reorder-imports"])
```

#### Solution 3 (for scripts that need to run at trunk)

If you write a script for which the API keeps changing, eg. you add / remove
flags to it, you can do something like this:

```
[aliases]
upload = ["util", "exec", "--", "bash", "-c", "python3", "-c", "$(jj file show -r 'trunk()' upload.py)"]
```

#### Decision: Trunk vs @

`@` and `trunk()` are the only two reasonable candidates as places to read from,
IMO. I personally believe that if only one option is available, `trunk()` would
be much more appropriate, for the reasons specified above.

However, @pmetzger has pointed out that in a future world where Git isn’t the
backend, this decision may come back to bite us (as build tools may be checked
in to the build).

For now, we will only support reading from the working copy (`@`), but in the
future, we may consider supporting other revisions.

To achieve this, there are several methods. I'll list some of them, but they
don't need review, as they are out of scope of this proposal.

The simplest that comes to mind is a `--managed-config-from-revision` flag or
config option.

Alternatively, a more general approach could be to create some kind of include
directive in `config.toml` files, like so (this has already been discussed and
rejected before, so it may not be viable).
```
includes = [
    {
        path = "other_config.toml",
        revision = "trunk()",
    },
]
```

#### Read from filesystem or from repo objects?

The tradeoffs are:
* If we read it from the filesystem, then it will not be read if the path is
  not tracked, e.g. because you have done `jj sparse --clear --add lib` or
  because the file matches .gitignore and is not yet tracked.
  * This could be mitigated by always including `.config/jj` in `jj sparse`
* If we read it from the repo objects, then changes will not apply to the next
  command unless we load this config after snapshotting
* Reading from the filesystem is faster
* mtime is only available when reading from the filesystem

Reading from the filesystem seems strictly superior, given the mitigation
strategy available, so we will go ahead with that for now. If we ever decide to
support reading from `trunk()`, however, we will need to support reading from
the repo.

[Chromium Formatter Config]: https://source.chromium.org/chromium/chromium/src/+/main:tools/jj/config.toml;l=47-96;drc=080c978973f87ff2a1cfa514a13285baeaf3eedc
[Chromium Pre-upload Checks]: https://source.chromium.org/chromium/chromium/src/+/main:tools/jj/upload.py;drc=96f39fbbb720ca391d43cbb199a85af7d3309dd3
[Chromium Syncing Scripts]: https://source.chromium.org/chromium/chromium/src/+/main:tools/jj/sync.py;drc=6ff08dcdd1fdeb1654a4d3da81d8adaeae4bbbf7
