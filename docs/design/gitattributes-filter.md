# Supporting the `filter` Git Attribute in Jujutsu

**Authors**: [Kaiyi Li]

**Summary**: This design proposes integrating the `filter` `gitattributes` into
jj's local working copy logic to enable arbitrary content conversion during
snapshot (clean) and update (smudge). It introduces new configuration settings
to define these filters, enabling features like `git-lfs` and `git-crypt`
support.

## Context and Scope

The `filter` attribute in git allows users to specify a driver that processes
file content.

- **Clean**: Run when adding files to the index (in `jj`, when snapshotting to
  the store). It typically converts a working copy file to a version suitable
  for the store (e.g., `git-lfs` converts large file content to a pointer file).
- **Smudge**: Run when checking out files to the working copy (in `jj`, when
  updating the working copy). It converts the content from the store back to a
  usable working copy file (e.g., `git-lfs` converts a pointer file to the
  actual large file content).

Supporting this feature is essential for compatibility with tools like [git-lfs]
(see [user request]) and [git-crypt] (see [user request][1]).

This design focuses on the integration of the `filter` attribute within `jj`'s
`local_working_copy` module. We will also introduce a new dedicated `filter`
module and types to handle filter functions.

### Terminology

#### Filter Related

We use the same terminology as the [git attributes document].

- **Filter Driver**: A command defined in the configuration that handles the
  smudge and clean operations.
- **Clean**: The operation to convert working copy contents to contents to be
  stored in the `Store`.
- **Smudge**: The operation to convert contents from the `Store` to working copy
  contents.

We also hold the same assumption for the `clean` and `smudge` operations as
[`git`][git attributes document].

> For best results, `clean` should not alter its output further if it is run
> twice ("clean→clean" should be equivalent to "clean"), and multiple `smudge`
> commands should not alter `clean`'s output ("smudge→smudge→clean" should be
> equivalent to "clean").

#### Jujutsu

- **Store**: The actual storage in the VCS, e.g. a git tree.

#### `gitattributes` file

A `gitattributes` file is a simple text file that gives attributes to pathnames.

Each line in `gitattributes` file is of form:

``` gitattributes
pattern attr1 attr2 ...
```

That is, a pattern followed by an attributes list, separated by whitespaces.

#### State

Each attribute can be in one of these states for a given path:

- Set

  The path has the attribute with special value "true"; this is specified by
  listing only the name of the attribute in the attribute list.

- Unset

  The path has the attribute with special value "false"; this is specified by
  listing the name of the attribute prefixed with a dash - in the attribute
  list.

- Set to a value

  The path has the attribute with specified string value; this is specified by
  listing the name of the attribute followed by an equal sign = and its value in
  the attribute list.

- Unspecified

  No pattern matches the path, and nothing says if the path has or does not have
  the attribute, the attribute for the path is said to be Unspecified.

!!! note

    `.gitattribute` files can use the `!` prefix to set an attribute to the Unspecified state:

    > Sometimes you would need to override a setting of an attribute for a path to `Unspecified` state. This can be done by listing the name of the attribute prefixed with an exclamation point `!`.

### Non-goals

- **Merge and diff**: Similar to the EOL design, we do not consider how filters
  affect merge, or diff in this design. `jj show`, `jj file show`, and `jj diff`
  will show and calculate the diff from the contents from the store. `jj new`
  and `jj rebase` will still resolve the conflicts based on the contents from
  the store.

- **Conflict handling**: We apply the filter conversion at the same place as the
  EOL conversion for conflicts: after the conflicts are materialized, i.e., we
  call the filter driver with conflict markers. We make such decision for
  simplicity. And if we want to improve, and apply the EOL and filter conversion
  on each side of the conflict, it will be a separate issue/design.

- **`git add --renormalize`**: We do not support re-normalizing files that are
  not changed in the working copy if the filter configuration changes. e.g., if
  `gitattributes` files are changed to opt-in a new file, `a.txt` to a filter in
  a working copy, and `a.txt` is not modified in the working copy, the clean
  filter still won't apply to `a.txt` in the store until `a.txt` is touched, and
  the smudge filter won't apply to `a.txt` on the disk until `a.txt` is checked
  out again. The behavior is the same when the filter config is changed: the new
  setting is only applied to files modified in the current working copy and/or
  on the next update(checkout). This behavior is consistent with `git` and
  `jj`'s existing EOL settings. However, such limitation will be well documented
  and a new issue will be opened to discuss whether `jj` need the
  `--renormalize` feature.

- **Long Running Filter Process**: We do not support the [long-running filter
  process protocol], i.e., the `filter.<name>.process` git config, in this
  design. We will spawn a new process for each file. This is deferred to future
  work.

- **Robust Process Management**: We do not implement the following robust child
  process management features:

  - Kill the child process if the parent `jj` process is terminated.
  - Set a timeout for the filter process.
  - Retry if the filter process fails.

  However, we **will** ensure that `jj` waits for the child process to complete
  to avoid resource leak, even if the filter process fails.

- **Reading gitattributes**: The mechanism for reading `gitattributes` files is
  covered in the [gitattributes design].

- **Reading and respecting the git filter configs**: We don't discuss using the
  existing git filter configs in this design, e.g. `filter.<name>.clean`,
  `filter.<name>.smudge`, and `filter.<name>.required`, for simplicity. `jj`
  should develop a consistent method to pull in git configs, and that's tracked
  in other issues: https://github.com/jj-vcs/jj/issues/4048, and
  https://github.com/jj-vcs/jj/issues/7455. Nonetheless, we do discuss `jj`'s
  equivalent user settings of those git configs.

- **Cancellation safety**: When the future returned by new async interfaces is
  dropped before the future is ready, the child process and the worker thread
  won't be terminated. However, if the future is ready, regardless of whether it
  resolves to `Ok` or `Err`, we make the best effor to ensure the child
  processes and worker threads have exited, i.e., no resources are leaked.

### Goals/Requirements

- Introduce a boolean `git.filter.enabled` setting as a killer switch on whether
  `filter` gitattributes conversion should happen. When the value is `false`,
  the implementation shouldn't read any `gitattributes` files and won't apply
  any filter conversion so that the user who doesn't need this feature pays
  little cost if not zero. It also prevents unexpected influence on `google3`
  repo, where there are many `gitattributes` files, but we don't expect it to
  have any effects. In addition, it's a "safe mode". In case a required filter
  driver doesn't work properly, e.g., always fails, almost no `jj` commands can
  run. Setting `git.filter.enabled` to `false` allows `jj` commands to work
  again. The default value is `false`. While this feature is probably only meant
  to be used in git repos, we also won't discourage the user from using the
  feature outside git repos.
- Introduce `git.filter.drivers.<name>.clean`,
  `git.filter.drivers.<name>.smudge`, and `git.filter.drivers.<name>.required`
  settings to define filter drivers.
- Support interpolating the relative file path in the command, where `$path`
  appears. This is similar to `%f` in the `filter.<name>.clean` git config.
  `$path` is used, because the settings of the fix tool, the external diff tool,
  and the external merge tool use `$path`.
- Support the `filter` attribute in `gitattributes`.
  - If `filter` is set to a string value, look up the corresponding driver in
    the settings.
  - If the driver is found, apply the `clean` command on snapshot and the
    `smudge` command on update. Otherwise, no conversion is applied silently. We
    should report the undefined filter driver in the tracing log for debugging.
- Ensure the order of conversion matches git:
  - **Snapshot**: The working copy file is first converted with the filter
    driver (if specified and corresponding driver defined), and then finally
    with EOL conversion (if specified and applicable).
  - **Update**: The content from the store is first converted with EOL
    conversion (if specified and applicable), and then fed to the filter driver
    (if specified and corresponding driver defined).
- Execute filter commands using `std::process::Command`, piping content via
  stdin/stdout.
- Ensure to clean up the resource: always wait for the child process to exit.
  When the returned future is ready, make the best effort to ensure that the
  child processes and the worker threads have exited.
- Handle command failure:
  - **Snapshot**: If a required filter fails, the operation, e.g., `jj log`,
    fails, and the current change in the store won't be updated. This is
    catastrophic, so we must ensure the error message is clear on what happens.
    If the filter is not required, the filter command failure results in a no-op
    (passthru) with a warning message on every filter command that fails just
    like `jj fix`. However, the exit code of the original command will be 0,
    which is different from `jj fix`.
  - **Update**: If a required filter fails, the operation fails, and the exit
    code of the `jj` command will be non-zero. A clear error message will
    indicate the user that it's a filter failure, and `jj` will leave the
    working copy [stale]. If the filter is not required, failure results in a
    no-op (passthru) with a warning message on every failure command just like
    `jj fix`. However, the exit code of the original command will be 0, which is
    different from `jj fix`.
- If no filter is applicable to the file, the file content should not be read
  when calling the filter conversion functions.
- Async runtime agnostic. Particularly, the functions shouldn't panic if it's
  called within the tokio async runtime or outside of the tokio async runtime.
- Won't block the async runtime worker thread, i.e., `Future::poll` will
  properly return when it's waiting for the child process to complete.
- Almost 100% test coverage. We strive for a very high test coverage, but it's
  difficult to test certain failures, e.g., we fail to create a worker thread.
- The implementation must be loosely coupled with `gitattributes`, so that in
  the future, if we decide to implement a general filter feature (hooks to
  modify contents on check in and checkout), we only need minimal modification.

## Open questions

1.  Should we move `CommandNameAndArgs` to the `lib` crate, so that the filter
    feature can share the same logic with the fix tool and the diff tool when
    parsing the `smudge` and `clean` settings?
2.  Do we prefer the method names to be `convert_for_snapshot`,
    `convert_for_update` or `convert_to_store`, `convert_to_working_copy` or
    `convert_to_store`, `convert_to_disk`? If latter, do we want to change the
    naming in the `eol` module?
3.  If the filter child process exits with 0 status code before we complete
    writing to stdin, should we treat as if this invocation fails, i.e., if it's
    a required filter, the jj command fails; if it's not a required filter, the
    filter is treated as a no-op (passthru)?
4.  Should we consider non-blocking implementation as a requirement? This adds
    complexity to the implementation.

## Overview

The example implementation can be found at
https://github.com/jj-vcs/jj/pull/8719. Feel free to skip the rest of the doc if
the actual implementation is more useful than description.

- The `FilterNameProvider` trait is implemented for `GitAttributes` to query the
  filter name associated with a path by searching the `filter` attribute.
- If a filter is specified and configured, the new
  `FilterStrategy::convert_to_store` and
  `FilterStrategy::convert_to_working_copy` will invoke the filter command.
- The filter command execution will be handled by a helper function that manages
  the subprocess and pipes.
- If a required filter fails on snapshot or update, `SnapshotError` or
  `CheckoutError` carries the file path and the filter name that fail. The
  `SnapshotError` or `CheckoutError` is then converted to a user `CommandError`
  with detailed message on the filter name and the file path, which should give
  enough information to the user on the error.
- If optional filters fail on snapshot or update, `SnapshotStats` or
  `CheckoutStats` carries the file paths that fail. `print_checkout_stats` or
  `print_snapshot_stats` then writes the warning message which tells the user
  what files are not converted as expected.

``` mermaid
classDiagram
    class FilterSettings {
        <<dataType>>
        +enabled: bool
        +drivers: HashMap~String, FilterDriver~
    }

    class FilterDriver {
        <<dataType>>
        +clean: Option~Vec~String~~
        +smudge: Option~Vec~String~~
        +required: bool
    }

    class FilterNameProvider {
        <<interface>>
        +get_filter_name(path: RepoPath) -> Result~Option~BString~~ 
    }

    class FilterStrategy {
        -filter_settings: FilterSettings
        -repo_root: PathBuf
        -proc_async_adapter: ProcessAsyncAdapter
        +convert_to_store(contents: AsyncRead, path: RepoPath, filter_name_provider: FilterNameProvider) Result~AsyncRead~
        +convert_to_working_copy(contents: AsyncRead, path: RepoPath, filter_name_provider: FilterNameProvider) Result~AsyncRead~
    }

    class ProcessAsyncAdapter {
        <<interface>>
        +spawn_and_wait_with_output(stdin_contents: &[u8], command: Command) Result~Output~
    }

    class FileSnapshotter {
        ~write_path_to_store(...) Result
    }

    class TreeState {
        -write_file(...) Result
    }

    %% Composition Edges (Ownership)
    %% The filled diamond goes on the side of the Owner
    FilterStrategy *-- "1" FilterSettings
    FilterStrategy *-- "1" ProcessAsyncAdapter
    FilterSettings *-- "*" FilterDriver : (via HashMap)

    %% Dependency Edges (Usage)
    TreeState ..> FilterStrategy : uses
    FileSnapshotter ..> FilterStrategy : uses
    FilterStrategy ..> FilterNameProvider
```

## Design

### Configuration

We introduce a new `git.filter.enabled` settings as a kill switch for filter
conversion. If it's set to `false`, filter conversion won't read the
gitattributes, and won't read the file.

We introduce a new `git.filter.drivers` setting to define filter
drivers. It's a new toml table. This acts the same as the git
`filter.drivers.<driver>.clean`, `filter.drivers.<driver>.smudge`, and
`filter.drivers.<driver>.required` configs.

``` toml
git.filter.enabled = true

[git.filter.drivers.lfs]
clean = ["git-lfs", "clean", "--", "$path"]
smudge = ["git-lfs", "smudge", "--", "$path"]
required = true
```

- `$path` will be replaced with the repo-relative path of the file being
  converted, e.g., the `git-lfs` filter driver needs this parameter. The path
  will use `/` as the separator, regardless of the platform, i.e., the path
  format is the same as the internal representation, which is the same as the
  fix tool.

We will introduce a `FilterSettings` struct to parse and store these
configurations.

``` rust
#[derive(Debug, PartialEq, Eq, Copy, Clone, serde::Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct FilterDriver {
    #[serde(default)]
    pub clean: Vec<String>,
    #[serde(default)]
    pub smudge: Vec<String>,
    #[serde(default)]
    pub required: bool,
}

pub struct FilterSettings {
    pub enabled: bool,
    pub drivers: HashMap<BString, FilterDriver>,
}

impl FilterSettings {
    pub fn try_from_settings(user_settings: &UserSettings) -> Result<Self, ConfigGetError> {
        ...
    }
}
```

We implement `try_from_settings` in a way very similar to [get_tools_config]:

1.  Retrieve `enabled` using
    `user_settings.get_bool("git.filter.enabled")`.
2.  Retrieve `drivers`:
3.  Use `user_settings.table_keys("git.filter.drivers")` to obtain the
    names of drivers.
4.  For each name, create the path `git.filter.drivers.<name>`.
5.  Call `user_settings.get::<FilterDriver>(path)` to retrieve the driver
    configuration.
6.  Collect the drivers into the `drivers` hash map with the names.

The `serde(default)` attributes will guarantee that `required` is default to
`false`, and `clean` and `smudge` are default to empty vectors - we treat empty
arrays the same as if the keys are missing.

We can't use [CommandNameAndArgs] here for `clean` and `smudge`, because
`CommandNameAndArgs` is defined in the `cli` crate.

### The `FilterStrategy` interfaces

We will introduce a new `filter` module and a `FilterStrategy` type to handle
filter conversions, similar to `TargetEolStrategy`.

``` rust
#[async_trait]
pub trait FilterNameProvider: Send {
    async fn get_filter_name(&self, path: &RepoPath) -> Result<Option<BString>, FilterError>;
}

pub struct FilterStrategy {
    settings: FilterSettings,
}

impl FilterStrategy {
    pub fn new(settings: FilterSettings) -> Self {
        Self { settings }
    }

    pub async fn convert_to_store<'a, F>(
        &self,
        contents: impl AsyncRead + Send + Unpin + 'a,
        path: &RepoPath,
        filter_name_provider: &F,
    ) -> Result<(Box<dyn AsyncRead + Send + Unpin + 'a>, Option<IgnoreReason>), FilterError>
    where
        F: FilterNameProvider + Unpin + 'a,
    {
        ...
    }

    pub async fn convert_to_working_copy<'a, F>(
        &self,
        contents: impl AsyncRead + Send + Unpin + 'a,
        path: &RepoPath,
        filter_name_provider: &F,
    ) -> Result<(Box<dyn AsyncRead + Send + Unpin + 'a>, Option<IgnoreReason>), FilterError>
    where
        F: FilterNameProvider + Unpin + 'a,
    {
        ...
    }
}
```

- `contents`: The input content stream. It can be the original contents of a
  file, or the contents after EOL conversion, e.g., on update. We use
  `AsyncRead` because `TargetEolStrategy` accepts and returns `AsyncRead`, so
  it's easier to chain the filter conversion and the EOL conversion with the
  `AsyncRead` type.
- `path`: The path of the file, used for `$path` substitution in the filter
  command, and query the filter name from `FilterNameProvider`.
- `filter_name_provider`: An async trait that returns the name of the filter
  associated with a file. `FilterNameProvider::get_filter_name` is only called
  if `FilterSettings::enabled` is true. We introduce a `FilterNameProvider`
  trait, so that the filter feature is decoupled from gitattributes. In the
  future, if we obtain the filter name in a way other than gitattributes, we
  just need to provide a new implementation of `FilterNameProvider`. The
  implementation of `FilterStrategy` methods have no direct connections to any
  `gitattributes` types.
- The `Option<IgnoreReason>` is `Some` when no filter conversion is applied to
  the file. It is used to generate the warning message when the optional filter
  command fails.

**Implementation**:

Both methods follow a similar pattern:

1.  Check if filters are enabled in `settings`. If not, return `contents` as is.
2.  Call `FilterNameProvider::get_filter_name` to retrieve the filter name.
3.  Resolve the `FilterDriver` based on the filter name and `settings` (see
    [Filter Resolution]).
4.  If no driver is resolved, return `contents` as is.
5.  Otherwise, a driver is found:

- For `convert_to_store`, use the `clean` command.
- For `convert_to_working_copy`, use the `smudge` command.

6.  [Construct] the command. If no command is constructed, it means the command
    is not specified, e.g., `convert_to_store` is called, but the driver has
    only `smudge`, but no `clean` command, and we return `contents` as is.
7.  Execute the command(see [Process Execution]). We introduce a dedicated
    `ProcessAsyncAdapter` trait for this. Essentially, we pass `contents` as
    stdin to the child process, and return the value read from the process
    stdout as the new `AsyncRead`. Using a separate trait allows us to mock with
    a test type that doesn't spawn an actual process for unit testing.
8.  If `ProcessAsyncAdapter::spawn_and_wait_with_output` returns an error or the
    status of the child process is not success:

- We leave a tracing log with the path to the file, the driver name, and the
  command to execute for easier debugging.
- If the filter is required, i.e., `FilterDriver::required` is `true`, return an
  error.
- If the filter is not required, i.e., `FilterDriver::required` is `false`,
  return `contents` as is.

Note that `FilterStrategy` does not handle EOL conversion. The caller is
responsible for chaining `FilterStrategy` and `TargetEolStrategy` in the correct
order (Disk -\> Clean -\> EOL -\> Store, and Store -\> EOL -\> Smudge -\> Disk).

### Filter Resolution

1.  When implementing `FilterNameProvider` for `GitAttributes`, if the attribute
    state is set to a value, i.e., `Status::Value` (e.g., `filter=lfs`), the
    value is returned (e.g., `b"lfs"`). Otherwise, `None` is returned.
1.  In `FilterStrategy`, we look up the filter name from
    `FilterSettings::drivers` (e.g., `"lfs"`). The type of the keys of
    `FilterSettings::drivers` is `String`, because the toml setting file must be
    UTF-8 encoded, so the valid driver name must be a valid UTF-8 string. If the
    filter value is not a valid UTF-8 string, we behave as if the filter driver
    is not found.
2.  If found, the value is the `FilterDriver` to use.
3.  If `FilterNameProvider::get_filter_name` returns `None`, i.e., the attribute
    is `Unset`, `Unspecified`, or `Set`, or if the driver is not found, do not
    apply any filter.

### Command Construction

We introduce a function to generate the `std::process::Command` (including the
arguments) used to spawn the child filter process.

``` rust
impl FilterStrategy {
    fn create_filter_command(&self, command: &[String], path: &RepoPath) -> Option<Command> {
        ...
    }
}
```

The implementation is similar to how `jj fix` is implemented in
[CommandNameAndArgs::to_command_with_variables] and [run_tool].

- Executable: If the command array is empty, the filter is not specified,
  otherwise, the first element is the path to the executable.
- Arguments: The rest of the elements are the arguments.
- Argument `$path` interpolation: The `$path` sequence in the arguments will be
  replaced with the path of the file being processed. The path of the file is
  obtained from the `path` parameter of `FilterStrategy::convert_to_*`. To
  perform the replace, we use [String::replace]. To convert the `RepoPath` to
  `String`, we use `RepoPath::as_internal_file_string`.
- The current working directory is set to the working copy root stored in
  `FilterStrategy`.
- All of `stdin`, `stderr`, and `stdout` will be set to `Stdio::piped()`.
  - `stdin`: Send the contents to be converted.
  - `stdout`: Receive the converted contents from the filter driver command.
  - `stderr`: Receive the error messages from the filter driver command. We will
    log the message if the log level is tracing, so that it's easier for the
    filter driver developer to debug.

### Process Execution

This section describes the implementation details of the
`ProcessAsyncAdapter::spawn_and_wait_with_output` function:

``` rust
#[async_trait]
impl ProcessAsyncAdapter for StdChildAsyncAdapter {
    async fn spawn_and_wait_with_output(
        &self,
        stdin_contents: &[u8],
        command: &mut Command,
    ) -> Result<Output, CommandError> {
        ...
    }
}
```

We use `std::process::Command::spawn` to execute the filter command, similar to
how `jj fix` is implemented in [run_tool][2]. The passed in `command` should
have been set per the [Command Construction][Construct] section. If we fail to
spawn the process, we return an error.

To send the original contents to the stdin, and read the converted contents from
the stdout, we spawn 2 threads to avoid deadlocks, and follow [this example].
However, we change the example slightly because the current thread may be in an
async context, and we want to avoid blocking:

- A stdin worker thread is spawned where we send the contents to the stdin of
  the filter process. We use a separate thread, because `write_all` can be
  blocking. We also use a `tokio::sync::oneshot::channel` to notify the original
  thread after the write completes successfully. If an error is encountered when
  writing, we write a tracing log about the error.

- Another output worker thread is spawned to call `Child::wait_with_output` and
  use `tokio::sync::oneshot::Sender::send` to send back the
  `std::process::Output` returned by `Child::wait_with_output`. We use a
  separate thread, because `Child::wait_with_output` is blocking.

- The current thread uses `tokio::sync::oneshot::Receiver` to receive the stdin
  worker thread success message and the `Output` data from the output worker
  thread in a non-blocking way by awaiting on `tokio::sync::oneshot::Receiver`.
  If any of the worker thread fails, awaiting on the corresponding channel
  results in an `Err`, because the worker thread drops the sender side without
  sending anything.

- If the stdin worker thread doesn't complete the write successfully(i.e. panics
  or fails), `stdin` can be dropped[^1] before all the contents are sent. In
  this case, the filter process receives EOF before it receives all the
  contents, and can exit with 0, but the output contains the filtered results
  generated from only a portion of the original contents. It's also possible
  that the filter process closes the stdin pipe before it reads everything on
  purpose, because it doesn't need to read all the contents. In this case, the
  write operation on the stdin worker thread fails legitimately. However, to be
  conservative, we always return `Err` in this case.

- If the output worker thread doesn't complete successfully(either panics[^2] or
  fails). We don't receive the output contents from the child filter process, we
  return `Err`.

- We use [Builder::spawn_scoped] or [Builder::spawn] to create the 2 worker
  threads with thread names and handle the case where we fail to create a
  thread. If we fail to create either thread, we make the best effort to kill
  the spawned child process before we panic, so that when the future is ready,
  it's likely that both the worker threads and the child process have exited,
  and no resources are leaked.

  - If `Child::kill` returns `Ok` we wait for the process in the original thread
    before we return. Waiting on a killed process should almost always returns
    immediately, so we just block the async runtime here. If we unfortunately
    fail to create worker threads, and the child filter process can't exit in
    time[^3], let's just hang for simplicity. If a user actually hits this
    issue, we can try to add timeout on waiting[^4] for the killed process.
  - If `Child::kill` returns `Err`, which is unlikely, we leave a tracing log,
    and we don't bother waiting for the child process, and just panic.

  We will also leave comments to future maintainers that we should avoid
  returning, panic, and adding await points after we spawn the process and
  before the output worker thread is spawned to avoid zombie processes. After
  the output worker thread is spawned, it is guanranteed that the child process
  is properly waited.

- Note that we always wait for the output worker thread first, then the stdin
  worker thread, so that when the function returns(particularly bails on error),
  the filter child process has exited, i.e., if the filter child process hangs,
  this function also hangs. For the same reason, we will leave a comment to warn
  future maintainers to avoid adding code between spawning the output worker
  thread, and the await on the `Child::wait_with_output` receiver on the
  original thread, because any panics in between can "leak" a hanging child
  process[^5].

- The implementation is not strictly cancellation safe: if the returned future
  is dropped after the first poll, but before the filter child process
  completes, the filter child process won't be killed, the 2 worker threads
  won't be terminated, and can result in a leak if the child process hangs.
  However, because the first await point is after the output worker thread is
  created, the child process is properly waited on the output worker thread, so
  we also don't result in a zombie process. We will document this behavior on
  this function and the `FilterStrategy::convert_to_*` functions, and we won't
  implement killing the child process on drop for simplicity.

- The implementation must be async runtime agnostic, i.e., can run in the tokio
  runtime and outside tokio runtime without panic. This requirement prevents us
  from using `tokio::process::Command` directly, which [panics outside the tokio
  runtime].

- We drop the `JoinHandle` of the 2 worker threads and make them detached
  threads. Panics and errors are propagated to the caller thread by dropping the
  sender without sending anything.

### Integration points in `local_working_copy`

- `FilterSettings` is added to `TreeStateSettings`, and initialized in
  `TreeStateSettings::try_from_user_settings`.
- `FilterStrategy` is added to `TreeState`, and initialized at
  `TreeState::empty()`.
- `FilterStrategy::convert_to_store` is called at
  `FileSnapshotter::write_path_to_store` and
  `FileSnapshotter::write_file_to_store`.
- `FileStrategy::convert_to_working_copy` is called at `TreeState::write_file`
  and `TreeState::write_conflict`.

To handle the required filter failures, the information of the error will be
part of `SnapshotError`/`CheckoutError`. The information includes the file repo
path and the filter driver to generate a clear error message for the user.

- snapshot: A new `CommandError::from_snapshot_error` function is introduced to
  replace the existing `<CommandError as From<SnapshotError>>::from` function.
  `CommandError::from_snapshot_error` properly uses `RepoPathUiConverter` to
  render the path in the error message.
- update: A new `CommandError::from_checkout_error` function is introduced which
  properly uses `RepoPathUiConverter` to render the path in the error message.
  `cli_util::update_working_copy` and `cli_util::update_stale_working_copy` uses
  this new method to convert `CheckoutError` to `CommandError`. A new
  `RepoPathUiConverter` argument is added to the 2 caller functions.

For optional filter failures, the error message will be part of
`SnapshotStats`/`CheckoutStats`:

- In `FileSnapshotter::write_path_to_store`,
  `FileSnapshotter::write_file_to_store`, `TreeState::write_file`, and
  `TreeState::write_conflict`, where
  `FilterStrategy::convert_to_{store,working_copy}` are called, if the returned
  `IgnoreReason` is filter command failure, the path and the `IgnoreReason` are
  added to the new `SnapshotStats::unconverted_paths` and
  `CheckoutStats::unconverted_paths` fields respectively.
- The existing `cli_util::print_snapshot_stats`, and
  `cli_util::print_checkout_stats` functions are responsible to use
  `RepoPathUiConverter` to write the warning message, so that the user is
  informed what files are not converted as expected.

## Tests

### Unit Tests

- **Configuration**: Test parsing of `git.filter` settings, and the
  default values for different fields.
- `FilterStrategy`: implement a `TestProcessAsyncAdapter` type to mock the
  process creation routine, and verify that proper `std::process::Command` is
  created for different cases and `FileStrategy` handles various errors
  correctly.
  - When the filter feature is not enabled, gitattributes should not be read.
  - Test the case where the file should not be read: the filter attributes is
    not set to a value, the value of the filter attributes is not a valid UTF-8
    string, the filter value doesn't match the names of any filter drivers, the
    corresponding filter command is not set.
  - The executable is set correctly.
  - **Command construction**: Test that the executable is set properly; the
    arguments are correct; particularly, `$path` is substituted correctly; the
    working directory is set to the working copy root; and the stdin, stderr and
    stdout are set to pipe.
  - **Error handling**: If the status code is not success,
    `FilterStrategy::convert_to_*` should return error. If
    `TestProcessAsyncAdapter::spawn_and_wait_with_output` succeeds, and the
    status code is success, the converted contents should be returned.
    Otherwise, we test the return value for both a required filter, and an
    optional filter.
- `StdChildAsyncAdapter`: it's not trivial to create a fake program and depends
  on the fake program for unit tests. The logic in this type is mostly covered
  by `jj-lib` integration tests.

### Integration Tests

We will introduce a new test helper tool `fake-filter`, similar to
`fake-formatter`, that can be configured to perform simple transformations
(e.g., convert to ASCII upper case) and fail on demand.

- **Clean path**:
  - Configure `fake-filter` as a clean filter.
  - Snapshot a file.
  - Verify the stored content is transformed.
- **Smudge path**:
  - Configure `fake-filter` as a smudge filter.
  - Update (checkout) a file.
  - Verify the working copy content is transformed.
- **Failure handling (Required)**:
  - Configure `fake-filter` to fail with `required = true`.
  - Verify the operation fails gracefully and reports the error.
  - For update, makes sure that the working copy is left to be stale.
- **Failure handling (Not Required)**:
  - Configure `fake-filter` to fail with `required = false`.
  - Verify the operation succeeds and the content is passed through unchanged.

Those tests will be in both the `jj-cli` crate and the `jj-lib` crate. In the
`jj-cli` crate, the error message on stderr will be checked to make sure the
error message is clear.

In the `jj-lib` integration test, we further test the following cases:

- The filter child process closes stdin before it reads anything, and exits with
  0.
- The filter child process reads one byte and writes one byte in turn. This test
  makes sure that writing to stdin and reading from stdout won't result in
  deadlock.
- The filter child process won't exit until the parent process gives a signal.
  Then we poll the future returned by `FilterStrategy::convert_to_*` until
  `Poll::Pending` is returned, before we signal the child process to exit.
  Afterwairds, block on the future until it returns. If the implementation is
  blocking, the test should timeout because of deadlock.
- Test calling the `FilterStrategy::convert_to_*` functions inside and outside
  the tokio runtime. No panic should happen.

## Future Possibilities

- **`tokio::process::Command`**: We currently use `std::process::Command` for
  simplicity and consistency with `jj fix`. We could switch to
  `tokio::process::Command` for async execution, which might improve performance
  when handling many files concurrently.
- **Long Running Filter Process**: Implement the persistent process protocol to
  avoid spawn overhead.
- **Git Config Compatibility**: Read `filter.<driver>.*` from git config files
  directly.
- **Obtain the filter driver name outside gitattributes file**: in the future,
  we may be able to store arbitrary key value pairs associated to a file or a
  folder, this allows us to store filter name in such key value pairs. It is
  also possible to associate filesets to filter names in the setting, so that
  we don't rely on gitattributes files.

[^1]: If `jj-lib` is used in a binary crate with `panic = "unwind"`, when panic
    happens, the stdin handle sent to the stdin worker thread [will be dropeed]
    as a process of unwind, which results in an EOF on the reader side, i.e.,
    the filter child process.

[^2]: If `jj-lib` is used in a binary crate with `panic = "abort"`, when the
    output worker thread panics, the process just dies, it doesn't matter if the
    original thread knows if the output worker thread succeeds or not. If
    `jj-lib` is used in a binary crate with `panic = "unwind"`, when the output
    worker thread panics, the `tokio::sync::oneshot::Sender` [will be
    dropped][will be dropeed], and the original thread awaits on the `Receiver`
    will result in an error.

[^3]: While it's unlikely that a process can't exit on kill, it happens if the
    process is trapped somewhere in the kernel, e.g., on Linux waiting for some
    hardware IO which puts the process in an uninterruptible sleep state, or on
    Windows, a thread hangs inside a kernel driver.

[^4]: This can be implemented, but is not trivial. Rust `std` doesn't provide
    such capability. On Windows, we can use `WaitForSingleObject`. On Linux, we
    can poll the status of the process with `waitpid` and `WNOHANG`.

[^5]: Let's use an example to explain this: if we wait for the stdin thread
    first, then the output thread, and the stdin thread fails. We return an
    error before we wait for the output thread sends back any message. It's
    possible that the child process hangs forever, but the returned future is
    ready and resolved to an error. This is bad, because this can cumulate
    hanging process silently. The alternative, waiting the output thread first
    results in a future that is only ready after the child process exits and is
    waited, allowing the caller to fully control the number of living processes.

  [Kaiyi Li]: mailto:kaiyili@google.com
  [git-lfs]: https://git-lfs.com/
  [user request]: https://github.com/jj-vcs/jj/issues/80
  [git-crypt]: https://github.com/AGWA/git-crypt
  [1]: https://github.com/jj-vcs/jj/issues/53#issuecomment-1206624208
  [git attributes document]: https://git-scm.com/docs/gitattributes#_filter
  [long-running filter process protocol]: https://git-scm.com/docs/gitattributes#_long_running_filter_process
  [gitattributes design]: gitattributes.md
  [stale]: ../working-copy.md#stale-working-copy
  [get_tools_config]: https://github.com/jj-vcs/jj/blob/19527a06167a17801b48ceca33a7646b8ec4e2f3/cli/src/commands/fix.rs#L378-L420
  [CommandNameAndArgs]: https://github.com/jj-vcs/jj/blob/main/cli/src/config.rs#L806C10-L813
  [Filter Resolution]: #filter-resolution
  [Construct]: #command-construction
  [Process Execution]: #process-execution
  [CommandNameAndArgs::to_command_with_variables]: https://github.com/jj-vcs/jj/blob/19527a06167a17801b48ceca33a7646b8ec4e2f3/cli/src/config.rs#L859-L871
  [run_tool]: https://github.com/jj-vcs/jj/blob/19527a06167a17801b48ceca33a7646b8ec4e2f3/cli/src/commands/fix.rs#L286-L293
  [String::replace]: https://doc.rust-lang.org/std/string/struct.String.html#method.replace
  [2]: https://github.com/jj-vcs/jj/blob/19527a06167a17801b48ceca33a7646b8ec4e2f3/cli/src/commands/fix.rs#L288-L313
  [this example]: https://doc.rust-lang.org/std/process/index.html#handling-io
  [Builder::spawn_scoped]: https://doc.rust-lang.org/std/thread/struct.Builder.html#method.spawn_scoped
  [Builder::spawn]: https://doc.rust-lang.org/std/thread/struct.Builder.html#method.spawn
  [panics outside the tokio runtime]: https://github.com/tokio-rs/tokio/blob/4714ca168d6bd97193625657b0381e9b65a9ceff/tokio/tests/process_change_of_runtime.rs#L25-L34
  [will be dropeed]: https://doc.rust-lang.org/reference/panic.html#r-panic.unwind.destruction
