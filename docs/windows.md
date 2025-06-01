# Working on Windows

Jujutsu works the same on all platforms, but there are some caveats that Windows
users should be aware of.

## Line endings conversion

Jujutsu currently respects Git's [`core.autocrlf`][git-autocrlf][^1], but does
not currently honor `.gitattributes`. The line endings conversion won't be
applied to files detected as a binary files via a heuristics[^2]. Jujutsu may
make incorrect decision on whether line conversion should be applied to a file,
but currently, Jujutsu doesn't support configuring line endings conversion for
files. To workaround this issue, one should disable the line conversion.

> [!NOTE]
> If Jujutsu doesn't apply line endings conversion on correct files, you should
  disable the line conversion. See below.

[^1]: Jujutsu just reads Git's `core.autocrlf` config, and doesn't have its own
      settings, so to change the end of line conversion settings, one needs to
      make modification to the underlying git config.
[^2]: To detect if a file is binary, Jujutsu currently reads the first 8KB of
      the file and uses the algorithm from [`gitoxide`][gitoxide-is-binary].
[git-autocrlf]: https://git-scm.com/book/en/v2/Customizing-Git-Git-Configuration#_core_autocrlf
[gitoxide-is-binary]: https://github.com/GitoxideLabs/gitoxide/blob/073487b38ed40bcd7eb45dc110ae1ce84f9275a9/gix-filter/src/eol/utils.rs#L98-L100

To disable line conversion, set the `core.autocrlf` config to `false`.

```powershell
PS> git config core.autocrlf false

# Abandoning the working copy will cause Jujutsu to overwrite all files with
# CRLF line endings with the line endings they are committed with, probably LF
PS> jj abandon
```

This means that line endings will be checked out exactly as they are committed
and committed exactly as authored.

This setting ensures Git will check out files with LF line endings without
converting them to CRLF. You'll want to make sure any tooling you use,
especially IDEs, preserve LF line endings.


## Pagination

On Windows, `jj` will use its integrated pager called `streampager` by default,
unless the environment variable `%PAGER%` or the config `ui.pager` is explicitly
set. See the [pager section of the config docs](config.md#pager) for more
details.

If the built-in pager doesn't meet your needs and you have Git installed, you
can switch to using Git's pager as follows:

```powershell
PS> jj config set --user ui.pager '["C:\\Program Files\\Git\\usr\\bin\\less.exe", "-FRX"]'
PS> jj config set --user ui.paginate auto
```

## Typing `@` in PowerShell

PowerShell uses `@` as part the [array sub-expression operator][array-op], so it
often needs to be escaped or quoted in commands:

```powershell
PS> jj log -r `@
PS> jj log -r '@'
```

One solution is to create a revset alias. For example, to make `HEAD` an alias
for `@`:

```powershell
PS> jj config set --user revset-aliases.HEAD '@'
PS> jj log -r HEAD
```

## WSL sets the execute bit on all files

When viewing a Windows drive from WSL (via _/mnt/c_ or a similar path), Windows
exposes all files with the execute bit set. Since Jujutsu automatically records
changes to the working copy, this sets the execute bit on all files committed in
your repository.

If you only need to access the repository in WSL, the best solution is to clone
the repository in the Linux file system (for example, in
`~/my-repo`).

If you need to use the repository in both WSL and Windows, one solution is to
create a workspace in the Linux file system:

```powershell
PS> jj workspace add --name wsl ~/my-repo
```

Then only use the `~/my-repo` workspace from Linux.

[array-op]: https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_arrays?view=powershell-7.4#the-array-sub-expression-operator

## Symbolic link support

`jj` supports symlinks on Windows only when they are enabled by the operating
system. This requires Windows 10 version 14972 or higher, as well as Developer
Mode. If those conditions are not satisfied, `jj` will materialize symlinks as
ordinary files.

For colocated repositories, Git support must also be enabled using the
`git config` option `core.symlinks=true`.
