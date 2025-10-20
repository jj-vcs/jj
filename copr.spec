Name:           jj-cli
Version:        {{version}}
Release:        %{autorelease}
Summary:        A Git-compatible VCS that is both simple and powerful

License:        Apache-2.0
URL:            https://github.com/jj-vcs/jj
Source0:        https://github.com/jj-vcs/jj/archive/refs/tags/v%{version}.tar.gz

BuildRequires:  rust >= 1.88
BuildRequires:  cargo >= 1.88

Recommends:     git

%description
Jujutsu is a powerful version control system for software projects. You use it to get a copy of your code, track changes to the code, and finally publish those changes for others to see and use. It is designed from the ground up to be easy to use—whether you're new or experienced, working on brand new projects alone, or large scale software projects with large histories and teams.

Jujutsu is unlike most other systems, because internally it abstracts the user interface and version control algorithms from the storage systems used to serve your content. This allows it to serve as a VCS with many possible physical backends, that may have their own data or networking models—like Mercurial or Breezy, or hybrid systems like Google's cloud-based design, Piper/CitC.

%prep
%autosetup -C

%build
cargo build --release --locked

%install
install -Dm755 %{_builddir}/%{buildsubdir}/target/release/jj %{buildroot}%{_bindir}/jj

%files
%license LICENSE
%{_bindir}/jj

%changelog
%autochangelog
