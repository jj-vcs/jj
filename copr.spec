Name:           jj-cli
Version:        {{version}}
Release:        %{autorelease}
Summary:        A Git-compatible VCS that is both simple and powerful

License:        Apache-2.0
URL:            https://github.com/jj-vcs/jj
Source0:        https://github.com/jj-vcs/jj/archive/refs/tags/v%{version}.tar.gz

BuildRequires:  rust >= 1.88
BuildRequires:  cargo >= 1.88

Requires:       git

%description
Jujutsu is a powerful version control system for software projects. You use it to get a copy of your code, track changes to the code, and finally publish those changes for others to see and use. It is designed from the ground up to be easy to use—whether you're new or experienced, working on brand new projects alone, or large scale software projects with large histories and teams.

Jujutsu is unlike most other systems, because internally it abstracts the user interface and version control algorithms from the storage systems used to serve your content. This allows it to serve as a VCS with many possible physical backends, that may have their own data or networking models—like Mercurial or Breezy, or hybrid systems like Google's cloud-based design, Piper/CitC.

%prep
%autosetup -C
cargo fetch --locked

%build
cargo build --release --frozen --bin jj

OLD_PATH="$PATH"
export PATH="$PWD/target/release:$PATH"
COMPLETE=bash jj > jj.bash
COMPLETE=fish jj > jj.fish
COMPLETE=zsh  jj > jj.zsh
export PATH="$OLD_PATH"

./target/release/jj util install-man-pages .

%install
install -Dm755 %{_builddir}/%{buildsubdir}/target/release/jj %{buildroot}%{_bindir}/jj

install -Dm644 %{_builddir}/%{buildsubdir}/jj.bash %{buildroot}%{bash_completions_dir}/jj
install -Dm644 %{_builddir}/%{buildsubdir}/jj.fish %{buildroot}%{fish_completions_dir}/jj.fish
install -Dm644 %{_builddir}/%{buildsubdir}/jj.zsh  %{buildroot}%{zsh_completions_dir}/_jj

install -dm755 %{buildroot}%{_mandir}
cp -a %{_builddir}/%{buildsubdir}/man1 %{buildroot}%{_mandir}

%files
%license LICENSE
%{_bindir}/jj

%{bash_completions_dir}/jj
%{fish_completions_dir}/jj.fish
%{zsh_completions_dir}/_jj

%{_mandir}/man1/jj.1*
%{_mandir}/man1/jj-*.1*

%changelog
%autochangelog
