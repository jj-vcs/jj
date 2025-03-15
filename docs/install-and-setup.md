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

First make sure that you have a Rust version >= 1.84 and that the
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

First make sure that you have a Rust version >= 1.84. You may also need to run:

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

First make sure that you have a Rust version >= 1.84. Now run either:

```shell
# To install the *prerelease* version from the main branch
cargo install --git https://github.com/jj-vcs/jj.git --locked --bin jj jj-cli
```

or:

```shell
# To install the latest release
cargo install --locked --bin jj jj-cli
```


## Initial configuration

You may want to configure your name and email so commits are made in your name.

```shell
$ jj config set --user user.name "Martin von Zweigbergk"
$ jj config set --user user.email "martinvonz@google.com"
```

## Command-line completion

To set up command-line completion, source the output of
`jj util completion bash/zsh/fish`. Exactly how to source it
depends on your shell.

Improved completions are also available. They will complete things like
bookmarks, aliases, revisions, operations and files. They can be context aware,
for example they respect the global flags `--repository` and `--at-operation` as
well as some command-specific ones like `--revision`, `--from` and `--to`. You
can activate them with the alternative "dynamic" instructions below. They should
still complete everything the static completions did, so only activate one of
them. Please let us know if you encounter any issues, so we can ensure a smooth
transition once we default to these new completions.

!!! info "Why are the improved completions not the default?"

    To generate the dynamic completion script, execute `jj` with `$COMPLETE` set
    to the name of your shell (see examples below). See the upstream clap issue
    [#3166][clap] for the explanation.


### Bash

#### Standard

```shell
source <(jj util completion bash)
```

#### Dynamic

```shell
source <(COMPLETE=bash jj)
```

### Zsh

#### Standard

```shell
autoload -U compinit
compinit
source <(jj util completion zsh)
```

#### Dynamic

```shell
source <(COMPLETE=zsh jj)
```

### Fish

!!! note

    No configuration is required with fish >= 4.1 which loads dynamic completions by default.

#### Standard

```shell
jj util completion fish | source
```

#### Dynamic

```shell
COMPLETE=fish jj | source
```

### Nushell

```nu
jj util completion nushell | save completions-jj.nu
use completions-jj.nu *  # Or `source completions-jj.nu`
```

(dynamic completions not available yet)

### Xonsh

```shell
source-bash $(jj util completion)
```

(dynamic completions not available yet)

### Powershell

Insert this line in your $PROFILE file:
(usually $HOME\Documents\PowerShell\Microsoft.PowerShell_profile.ps1)

```shell
Invoke-Expression (& { (jj util completion power-shell | Out-String) })
```

(dynamic completions not available yet)

[cargo-binstall]: https://github.com/cargo-bins/cargo-binstall
[clap]: https://github.com/clap-rs/clap/issues/3166
[Homebrew]: https://brew.sh/
[macports]: https://ports.macports.org/port/jujutsu/
[pre-built binaries]: https://github.com/jj-vcs/jj/releases/latest
