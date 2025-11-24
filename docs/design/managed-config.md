# Repo-managed JJ configuration

Author: [Matt Stark](mailto:msta@google.com)

## Background

The design doc [Secure JJ Config](secure-config.md) introduces a mechanism, `metadata.binpb`, through which information about a repository / workspace can be stored. It also discusses how allowing an external user to have control over your config is a security risk.

## Overview

There is a need for a repository to impose requirements upon users. Examples of these things include, but are not limited to:

* Formatter configuration (eg. [chromium](https://source.chromium.org/chromium/chromium/src/+/main:tools/jj/config.toml;l=47-96;drc=080c978973f87ff2a1cfa514a13285baeaf3eedc))
* Pre-upload checks (eg. [chromium](https://source.chromium.org/chromium/chromium/src/+/main:tools/jj/upload.py;drc=96f39fbbb720ca391d43cbb199a85af7d3309dd3))
* Syncing scripts (eg. [chromium](https://source.chromium.org/chromium/chromium/src/+/main:tools/jj/sync.py;drc=6ff08dcdd1fdeb1654a4d3da81d8adaeae4bbbf7))
* Custom aliases / revsets documented by some kind of “getting started with <project>” document

It should be fairly obvious that there is a strong benefit to doing so. However, controlling a user’s config is sufficient to get root access to their machine, so we require a mechanism more complex than just blindly loading the config file.

This is currently achieved by projects such as chromium by instructing the user to symlink `.jj/repo/config.toml` to a file in the repository. This has several drawbacks:
* It doesn’t  work out-of-the-box. I need to manually symlink it
* It has no security guarantees. If I update the file in the repo, the user has no opportunity to review it.
* This prevents a user from having their own repo configuration on top of it.

## Objective 
* Create a new layer of configuration between user configuration and repo configuration.
  * This configuration will be stored in version control and henceforth be referred to as “managed” configuration.
* Implement it in a secure manner so that an attacker cannot take control over the managed config.

## Detailed Design
The managed configuration will be read from `$REPO/.config/jj/config.toml`. This is intentionally designed to be very similar to `$HOME/.config/jj/config.toml` for the user configuration.

Any data stored here this will be added to the `metadata.binpb` that was created in [secure config](secure-config.md) (that is, we will add additional fields to the `Metadata` struct). This will ensure that we don't suffer from the "zip file problem".

### Security
#### Trust levels
We will add the following fields to metadata:
```
enum TrustLevel {
    // There's no "optional" in protos. It just returns the zero value.
    UNSET = 0;
    // The user wishes to completely ignore the managed config file.
    DISABLED = 1;
    // In trusted mode, we directly read from the managed config file.
    // This presents a security risk, so the user is expected to only do this
    // for a repo they trust.
    TRUSTED = 2;
    // In notify mode, the user is expected to manually replicate any changes
    // they want from the managed config to the repo config.
    NOTIFY = 3;
}

message Metadata {
    // previous fields

    // The trust level associated with this repo.
    TrustLevel trust_level = 2;
    // The mtime last seen associated with the config.toml file.
    Timestamp last_modified = 3;
    // The content of the most recently seen config.toml file.
    string last_managed_config = 4;
}
```

Note that although the `Metadata` struct is stored for both the repo and the workspace, this will only be used for the repository, not the workspace.

#### User interface
Everything here only becomes relevant when a repo has a managed config. If it does not have a managed config, we skip everything here.

##### Unset trust
If a repository has unset trust, we first use `ui.can_prompt()` to determine whether we are in a TUI.
* If we are in a TUI, we ask the user what trust level they would like.
* If we are not in a TUI, we warn them that we are ignoring the repo config (we return `DISABLED`), and that they can run `jj` in a terminal to configure this.

##### Disabled
Nice and easy. We just completely ignore the managed config file.

##### Trusted
Nice and easy. We just read the managed config file.

##### Notify
Notify is not a part of the MVP, and will be added later, in order to keep the MVP simple.

Roughly speaking, we will need to:
* if `mtime(repo_config_file) < mtime(managed config)`
  * The repo config is "out of date"
  * We print a warning, potentially diffing `last_managed_config` with the actual managed config on disk.
* otherwise, the repo config isn't out of date
  * `last_modified = mtime(managed config)`
  * `last_managed_config = read(managed config)` (if mtime changed)

The notify option has a rather painful UX when it comes to keeping it in sync (particularly for a user who constantly switches between different branches synced to different versions), but I choose it for several reasons. The first is that the vast majority of users will simply select to blindly trust the repo. The only people who will choose this option are the very security conscious, and this is by far the most secure mechanism. Secondly, it is so much simpler than an approval based mechanism, as we don’t need to worry about things such as workspaces being synced to different places. It provides far fewer edge cases.

##### Manually changing
These options will also be able to be manually set via `jj config managed --disable/notify/trust`. This is not a part of the MVP.

### Where to read from

This is the trickiest part of the proposal. Consider the following workflow:

```
jj new main@origin
jj ...
jj new lts-branch@origin
```

There are some edge cases we need to consider:
* `lts-branch` may have existed before the config was added
* `lts-branch` may have a different copy of the managed config

The naive assumption would be that you want to read the config from `@`, as the config will always match the version of the code you're using. However, it turns out that some things want to refer to `@`, while others want to read the config from `trunk()`.

Consider several different use cases:
* My formatter was previously `clang-format --foo`, but the option `--foo` was deprecated in the latest version of `clang-format`
  * Here, you want to read from `trunk()`
* My formatter was previously `$repo/formatter --foo`, but the option `--foo` was deprecated in the latest version of `formatter`
  * Here, you want to read from `@`
* We decide to split long lines and add a new formatter (or pre-upload hook) config `formatter --line-length=80`
  * Here, you probably want to read from `@`
* We decide to add a pre-upload check that validates that all commit descriptions contain a reference to a bug
  * This should be applied to old branches as well, so you want `trunk()`
* We add a new helpful alias / revset
  * This should be applied to old branches as well, so you want `trunk()`
* We move our formatter
  * If it’s external, you want to read from `trunk()`
  * If it’s internal, you want to read from `@`

All in all, you can see a general pattern.
* If something refers to an in-repo tool, you **probably** want the config to be read from `@`
* Otherwise, you **probably** want to read from `trunk()`
* I say probably, because the split long lines example doesn’t conform to this rule.

#### Problematic examples

This is problematic with `trunk()` because if you add the `--reorder-hooks` and then checkout `lts-branch` it will incorrectly attempt to reorder imports

```
[fix.tools.rustfmt]
command = ["rustfmt", "--reorder-imports"]
```

##### Solution 1: formatter config

In practice, it is highly unlikely that a formatter config would be written that way. Far more likely, you would see an entry in `config.toml` like:

```
[fix.tools.rustfmt]
command = ["rustfmt"]
```

`.rustfmt.toml`:
```
reorder_imports = true
```

This simply works out of the box, since the formatter is reading the config from `@`'s `.rustfmt.toml`

##### Solution 2 (more general but convoluted): Wrapper script

As long as the formatter is in-repo, we can just write a wrapper script which does this for us.

```
[fix.tools.rustfmt]
command = ["rustfmt.py"]
```

`rustfmt.py`:
```
os.execv(["rustfmt", "--reorder-imports"])
```
#### Solution 3 (for scripts that need to run at trunk)

If you write a script for which the API keeps changing, eg. you add / remove flags to it, you can do something like this:

```
[aliases]
upload = ["util", "exec", "--", "bash", "-c", "python3", "-c", "$(jj file show -r 'trunk()' upload.py)"]
```

#### Decision: Trunk vs @
`@` and `trunk()` are the only two reasonable candidates as places to read from, IMO. I personally believe that if only one option is available, `trunk()` would be much more appropriate, for the reasons specified above.

However, @pmetzger has pointed out that in a future world where git isn’t the backend, this decision may come back to bite us (as build tools may be checked in to the build). This is already the case for some git repositories such as the android repo. To resolve this easily, I propose supporting both.

To achieve this, we would split the managed config file in two. We would now have:
* `$REPO/.config/jj/working_copy_config.toml`, which would always be read from `@`
* `$REPO/.config/jj/trunk_config.toml`, which would always be read from `trunk()`

Note that we will only implement one of these in the MVP, and choose to do the other later (we will probably do `@` first since it's simpler to implement).

## Alternatives considered
### Approval mechanisms
We considered an approval based mechanism where when you saw a config you hadn't seen before, you would appprove or reject it. It turned out to have a lot of edge cases though, and was extremely complex (for both the user and the implementer). For example, what happens if you approve some config, then sync to an old commit? What happens if you have two workspaces synced to different commits. What happenps if you want to approve some of the changes but not others?