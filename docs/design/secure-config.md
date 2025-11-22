# Secure JJ config

Author: [Matt Stark](mailto:msta@google.com)

# The problem

An attacker that has control over your jj configuration has full control over your system when you run specific commands. As an example, an attacker can have you enable the following repo config:

```
[fix.tools.foo]
command = ["malicious", "command"]
```

When a user then runs `jj fix`, this will run their malicious command and they can gain full control over your system. This can be achieved via zipping up a repo and sending it to the user, with the `.jj/repo/config.toml` file containing the above config (hence why this is colloquially known as the “zip file problem”).

There are plans to add features such as hooks to jj which will only make it easier for this to occur. For simplicity’s sake, we will assume that if an attacker has their configuration enabled on your system, it is compromised.

Assume any reference to repo config can equivalently be replaced with workspace configs. We will treat them in the same way.

## Threat model

This is not something that can be 100% defended against. Defense against all possible attack vectors is infeasible, so we will instead note all the attack vectors and what it would take to defend against them.

### Attack vector 1: No-knowledge attacker

1. The attacker creates a repo
2. The attacker runs `jj config set --repo fix.tools.foo ‘[“malicious”, “command”]’`
3. The attacker zips up their repo and sends it to the victim
4. The victim unzips the repo, make some changes, then run `jj fix`
5. They have now executed an arbitrary command on the victim’s system

This attack vector can be solved by ensuring that we can determine the user who created the repo.

### Attack vector 2: Basic replay attack

1. The victim uploads a zip file of a repository they have locally on their system
1. The attacker can now see  stored in the repository
2. The attacker runs `jj config set --repo fix.tools.foo ‘[“malicious”, “command”]’`
3. The attacker copies the victim’s cryptographic signature and puts it in their malicious repository.
4. The attacker zips up their repo and sends it to the victim
5. The victim unzips the repo at an arbitrary location, make some changes, then run `jj fix`
6. They have now executed an arbitrary command on the victim’s system

This attack vector can be solved by ensuring that we can determine the path that the repo was stored at.

### Attack vector 3: Replay attack with some social engineering to preserve paths

1. The victim uploads a zip file of a repository they have locally on their system at `/path/to/repo`
1. The attacker can now see any cryptographic signatures stored in the repository
2. The attacker runs `jj config set --repo fix.tools.foo ‘[“malicious”, “command”]’`
3. The attacker copies the victim’s cryptographic signature and puts it in their malicious repository.
4. The attacker zips up their repo, sends it to the victim, and instructs them to install it at `/path/to/repo`
5. The victim unzips the repo at `/path/to/repo`, make some changes, then run `jj fix`
6. They have now executed an arbitrary command on the victim’s system

This attack vector can be solved by making repository configuration untamperable.

### Attack vector 4: Extremely advanced replay attack with insecure code

1. The victim creates a repo
2. The victim runs `jj config set --repo fix.tools.foo = [“$repo/format.py”]`
3. The victim uploads a zip file of a repository they have locally on their system at `/path/to/repo`
1. The attacker can now see any cryptographic signatures stored in the repository
4. The attacker modifies `format.py` to be malicious
5. The attacker zips up their repo and sends it to the victim
6. The victim runs `jj fix`
7. They have now executed an arbitrary command on the victim’s system

This attack vector cannot feasibly be dealt with. It would require a signature of the transitive closure of files that can be accessed via jj configs to solve.

# Objective 

## Goals

* Prevent as many of the above attack vectors as possible
* Have minimal negative impacts on UX

## Non-goals (Optional)

* Use strategies such as sandboxing to mitigate damage
  * We could do this for formatters, for example, but then repo hooks would have the same problem
  * These options are not mutually exclusive

# Detailed Design

## Storing config out-of-repo

We will store per-repo configuration in `etcetera::BaseStrategy::config_dir().join(“jj”).join(“repos”).join(id)`. 

The structure of a repo will look like so:

```
$HOME/.config/jj/
  repos/
    abc123/
      metadata.binpb
      config.toml
  workspaces/
    def456/
      config.toml
      metadata.binpb
my-repo/.jj/
  workspace-id (contains "def456")
  workspace-config.toml (symlink to $HOME/.config/jj/workspaces/def456/config.toml)
  repo/
    repo-id (contains "abc123")
    config.toml (symlink to $HOME/.config/jj/repos/abc123/config.toml)
```
Metadata.binpb will refer to the following protobuf:

```proto
message Metadata {
  // This is used to distinguish between copies and moves.
  string path = 1;
}
```

The function to load repository configuration, will roughly speaking, look like:
```rust
enum ConfigLoadError {
    NoRepoId,
    NoConfig,
    PathMismatch,
}

fn load_repo_config_path(repo: Path) -> Result<PathBuf, ConfigLoadError> {
    let Ok(repo_id) = std::fs::read_to_string(repo.join("repo-id")) else return Err(NoRepoId);
    let repo_config_dir = config_dir.join("repos").join(repo_id);
    let Ok(metadaata) = Metadata::decode(std::fs::read(repo_config_dir.join("metadata.binpb"))) else return Err(NoConfig);
    if metadata.path != repo {
        return Err(PathMismatch)
    }
    Ok(repo_config_dir.join("config.toml"))
}
```

### No repo-ID
All new repos should have repo IDs, so this should mean that we have a repo created by an old version of jj.

To preserve backwards compatibility, we will introduce a period of auto-migration. The current plan is 12 jj versions (approximately 1 year). During this period, if a repo ID has not yet been generated, we will silently perform the following (order matters, to ensure failure halfway through doesn’t affect things):

1. Generate a repo ID `abc123`
2. Create `$HOME/.config/jj/repo/abc123/metadata.binpb`
3. Create `$HOME/.config/jj/repo/abc123/config.toml` as a copy of the original config file
4. Atomically generate a Repo-ID file containing `abc123`
5. Remove the original config file
6. Symlink the original config file to the new config file

After the migration period is over, we will:
* Add comments at the start of every line in step 3
* Print a warning to the user that their config has been migrated, but commented out, and that they need to run `jj config edit` to get the config back.

### No Config
This could occur, for example, if the user created a repository in linux, rebooted into windows on the same computer, and attempted to access that repo.

In this event, the user probably expects their config to be attached to the repository, and they expect it to still work on linux, so we will:
* Create a new repo with the same repo ID (to ensure that the config still works on windows)
* Print a warning that if there was any per-repo configuration, it is no longer available.

### Path mismatch
#### Secure
If we have a path mismatch, one of the following things have occurred:

* The repo has been moved
* The repo has been copied
* The user is a victim of a replay attack where the user replays the repo id

We can distinguish between the first two by attempting to check whether the old path still exists on disk.

* If the old path does not exist on disk, the repo has been moved, so we need to update the path in `metadata.binpb`
* If the old path does exist on disk, the repo has been copied, so we need to generate a new repo ID, and create a copy of the directory `.config/jj/OLD_REPO_ID` for the new repo ID

This prevents a situation where modifying one repo’s config modifies another as well However, this does not prevent a situation where you:
1. `cp -r original copy`
2. Make a config change to the original
3. Then run a jj command in the copy

In this example, the copy would only actually copy the config when you first ran the command in the config, so it would include the config change to the original. This is a minor UX annoyance, but:
* It's still relatively minor
* Users should generally use `jj workspace add` instead of copying the whole directory.

#### Insecure
Unfortunately, there is no way to distinguish copying / moving from a replay attack. The attacker, if they know a repo ID that exists on your system, can create a repo with the same repo ID. However, the fact that the config itself is stored out of repo inherently prevents simple replay attacks. In order for the attacker to exploit this, they would need to:

* Know your repo ID (requires uploading a zip file or something similar)
* Get lucky by the victim having a “risky” per-repo config
* Eg. fix.tools pointing to `$repo/formatter`
* Know how to exploit it
* Because your config file is stored out-of-repo, the attacker will likely not know none of this without some social engineering

We intentionally choose not to deal with this kind of attack in the initial version, and have no current intention to solve it in future versions either (as the UX impact would likely be much larger than any benefit to security).

We could potentially add this as an opt-in feature in the future, but it has dubious benefit, as the kind of user who would opt in to something like this is also the kind of user who would never upload their repo as a zip file.

## Attack vectors remaining

Because only the repo-id and workspace-id are stored in-repo, the only attack vector remaining is the replay attack I mentioned above.

## UX issues
* Copying the repo is essentially a symlink to an old config until you update it
* Multiple users on the same system would each have different per-repo configs
  * This can be solved by simply symlinking `$HOME/.config/jj` to `%APPDATA%/jj` (or vice versa) to solve this issue. You were probably doing this anyway with specifically the user config file instead of the directory.

# Alternatives considered
## Store the configs in-repo with an untamperable cryptographic signature

There are a few questions we would need to resolve here, all with significant drawbacks:

#### Do we include paths in the signature?
If we do, we introduce a whole bunch of additional annoying UX to the user when they move repos around.

If we don't, we leave ourselved exposed to additional attack vectors

#### How to sign the content of the repo config?
* We could not sign it at all, but that would leave ourselves exposed to additional attack vectors.
* We could sign the content of the repo config, but then when the user manually edits the file we have additional UX we need to introduce.
* We could store both the content and the signature in the repo protobuf, but the config would no longer exist on disk as a regular file, and thus you couldn't use standard tools to read and write the config.

All of these options were discussed in the original PR (#7761), which, unlike the current approach, introduced user interventions and a review process. The current approach, on the other hand, while it does have some extremely minor UX weirdness, has no such issues.
