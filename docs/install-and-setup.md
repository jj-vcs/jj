# Installation and setup


## Installation

### Download pre-built binaries for a release

There are [pre-built binaries] of the last released version of `jj` for
Windows, Mac, or Linux (the "musl" version should work on all distributions).

#### Cargo Binstall

If you use [`cargo-binstall`][cargo-binstall], you
can install binaries of the latest `jj` release from GitHub as follows:

```shell
# Will put the jj binary for the latest release in ~/.cargo/bin by default
cargo binstall --strategies crate-meta-data jj-cli
```

Without the `--strategies` option, you may get equivalent binaries that should
be compiled from the same source code.

!!! note

    If you'd like to install a prerelease version, you'll need to use one of the
    options below.

### Linux

#### From Source

First make sure that you have a Rust version >= 1.88 and that the
`build-essential` package is installed by running something like this:

```shell
sudo apt-get install build-essential
```

Now run either:

```shell
# To install the *prerelease* version from the main branch
cargo install --git https://github.com/jj-vcs/jj.git --locked --bin jj jj-cli
```

or:

```shell
# To install the latest release
cargo install --locked --bin jj jj-cli
```

#### Arch Linux
You can install the `jujutsu` package from the [official extra repository](https://archlinux.org/packages/extra/x86_64/jujutsu/):

```shell
pacman -S jujutsu
```

Or install from the [AUR repository](https://aur.archlinux.org/packages/jujutsu-git) with an [AUR Helper](https://wiki.archlinux.org/title/AUR_helpers):

```shell
yay -S jujutsu-git
```

#### NixOS

If you're on NixOS you can install a **released** version of `jj` using the
[nixpkgs `jujutsu` package](https://search.nixos.org/packages?channel=unstable&show=jujutsu).

To install a **prerelease** version, you can use the flake for this repository.
For example, if you want to run `jj` loaded from the flake, use:

```shell
nix run 'github:jj-vcs/jj'
```

You can also add this flake url to your system input flakes. Or you can
install the flake to your user profile:

```shell
# Installs the prerelease version from the main branch
nix profile install 'github:jj-vcs/jj'
```

#### Homebrew

If you use Homebrew, you can run:

```shell
# Installs the latest release
brew install jj
```

#### Gentoo Linux

`dev-vcs/jj` is available in the [GURU](https://wiki.gentoo.org/wiki/Project:GURU) repository.
Details on how to enable the GURU repository can be found [here](https://wiki.gentoo.org/wiki/Project:GURU/Information_for_End_Users).

Once you have synced the GURU repository, you can install `dev-vcs/jj` via Portage:

```shell
emerge -av dev-vcs/jj
```

#### openSUSE Tumbleweed

`jujutsu` can be installed from the official [openSUSE-Tumbleweed-Oss](http://download.opensuse.org/tumbleweed/repo/oss/) repository:

```shell
zypper install jujutsu
```

### Mac

#### From Source

First make sure that you have a Rust version >= 1.88. You may also need to run:

```shell
xcode-select --install
```

Now run either:

```shell
# To install the *prerelease* version from the main branch
cargo install --git https://github.com/jj-vcs/jj.git \
     --locked --bin jj jj-cli
```

or:

```shell
# To install the latest release
cargo install --locked --bin jj jj-cli
```

#### Homebrew

If you use Homebrew, you can run:

```shell
# Installs the latest release
brew install jj
```

#### MacPorts

You can also install `jj` via [the MacPorts `jujutsu`
port][macports]:

```shell
# Installs the latest release
sudo port install jujutsu
```

### Windows

First make sure that you have a Rust version >= 1.88. Now run either:

```shell
# To install the *prerelease* version from the main branch
cargo install --git https://github.com/jj-vcs/jj.git --locked --bin jj jj-cli
```

or:

```shell
# To install the latest release
cargo install --locked --bin jj jj-cli
```

via winget:

```shell
# To install the latest release via winget
winget install jj-vcs.jj
```

via scoop:

```shell
# To install the latest release via scoop
scoop install main/jj
```


## Initial configuration

You may want to configure your name and email so commits are made in your name.

```shell
$ jj config set --user user.name "Martin von Zweigbergk"
$ jj config set --user user.email "martinvonz@google.com"
```

## Command-line completion

Jujutsu provides 2 different command-line completion scripts.

### Standard completions

The standard completion script provides completions for `jj` subcommands and
options.

### Dynamic completions

The dynamic completion script provides completions for `jj` subcommands and
options, as well as additional completions, including bookmarks,
aliases, revisions, operations and files. Dynamic completions can be context
aware, for example they respect the global flags `--repository` and
`--at-operation` as well as some command-specific ones like `--revision`,
`--from`, and `--to`.

Dynamic completions are not the default/only option since the underlying engine
[is still labeled unstable][clap]. We expect to transition to them as the
default once the engine is stabilized. Please let us know if you encounter any
issues with dynamic completions.

!!! info "Which completion script should I use?"

    Generally, dynamic completions provide a much better completion experience.
    Although the underlying engine is deemed unstable, there have not been many
    issues in practice. Dynamic completions are the preferred option for many
    contributors and users.

    We recommend using the dynamic completion script, and falling back to the
    standard completion script if there are any issues.

<!-- The content tabs formatting below is optimized for the website and not for GitHub. -->

### Bash

=== "Standard"

    ```shell
    source <(jj util completion bash)
    ```

=== "Dynamic"

    ```shell
    source <(COMPLETE=bash jj)
    ```

### Zsh

=== "Standard"

    ```shell
    autoload -U compinit
    compinit
    source <(jj util completion zsh)
    ```

=== "Dynamic"

    ```shell
    source <(COMPLETE=zsh jj)
    ```

### Fish

!!! note

    No configuration is required with fish >= 4.0.2 which loads dynamic completions by default.

=== "Standard"

    ```shell
    jj util completion fish | source
    ```

=== "Dynamic"

    ```shell
    COMPLETE=fish jj | source
    ```

### Nushell

=== "Standard"

    ```nu
    jj util completion nushell | save completions-jj.nu
    use completions-jj.nu *  # Or `source completions-jj.nu`
    ```

### Xonsh

=== "Standard"

    ```shell
    source-bash $(jj util completion)
    ```

### Powershell

=== "Standard"

    ```powershell
    jj util completion power-shell | Out-String | Invoke-Expression
    ```

=== "Dynamic"

    ```powershell
    $env:COMPLETE = "powershell"
    jj | Out-String | Invoke-Expression
    Remove-Item Env:\COMPLETE
    ```

Insert the above into your `$PROFILE` file
(usually `$HOME\Documents\PowerShell\Microsoft.PowerShell_profile.ps1`).

!!! note

    Note that to execute scripts in PowerShell on Windows, including `$PROFILE`,
    the [execution policy][ExecutionPolicies] needs to be set to `RemoteSigned`
    at minimum.


[cargo-binstall]: https://github.com/cargo-bins/cargo-binstall
[clap]: https://github.com/clap-rs/clap/issues/3166
[Homebrew]: https://brew.sh/
[macports]: https://ports.macports.org/port/jujutsu/
[pre-built binaries]: https://github.com/jj-vcs/jj/releases/latest
[ExecutionPolicies]: https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_execution_policies
