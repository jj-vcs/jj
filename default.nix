{
  #
  # Stdenv
  #
  lib,
  stdenv,
  rustPlatform,
  stdenvAdapters,
  #
  # nativeBuildInputs
  #
  gzip,
  pkg-config,
  cmake,
  installShellFiles,
  #
  # BuildInputs
  #
  openssl,
  libgit2,
  libssh2,
  darwin,
  libiconv,
  #
  # NativeCheckInputs
  #
  gnupg,
  openssh,
  git,
  #
  # User input
  #
  gitRev ? "dirty",
}: let
  fs = lib.fileset;
  files = fs.difference (fs.gitTracked ./.) (fs.unions [
    ./.github
    ./.editorconfig
    ./.envrc.recommended
    ./.watchmanconfig
    ./AUTHORS
    ./README.md
    ./LICENSE
    ./GOVERNANCE.md
    ./SECURITY.md
    ./deny.toml
    ./mkdocs.yml
    ./mkdocs-offline.yml
    ./pyproject.toml
    ./uv.lock
    ./flake.lock
    (fs.fileFilter (f: f.hasExt "nix") ./.)
    (fs.fileFilter (f: f.name == "README.md") ./.)
  ]);
in
  rustPlatform.buildRustPackage.override {
    # Mold can run most places, but can only build ELF files.
    stdenv =
      if stdenv.hostPlatform.isElf
      then stdenvAdapters.useMoldLinker stdenv
      else stdenv;
  } (finalAttrs: {
    strictDeps = true;

    pname = "jujutsu";
    version = "unstable-" + gitRev;

    cargoLock.lockFile = ./Cargo.lock;

    cargoBuildFlags = ["--bin" "jj"];
    buildFeatures = ["packaging"];
    buildType = "release";

    doCheck = false;
    useNextest = true;
    checkType = finalAttrs.cargoBuildType;
    cargoTestFlags = [
      # Don’t build the `gen-protos` build tool when running tests.
      "-p"
      "jj-lib"
      "-p"
      "jj-cli"
    ];
    checkFlags =
      lib.optionals (lib.inPureEvalMode) [
        # Doesn't work in the sandbox
        "--skip"
        "test_git::test_push_bookmarks_deletion::use_git2_for_remote_calls"
      ]
      ++ lib.optionals (lib.strings.hasInfix "dirty" gitRev) [
        # Test fails if is marked dirty.
        "--skip"
        "test_global_opts::test_version"
      ];

    src = fs.toSource {
      root = ./.;
      fileset = files;
    };

    nativeBuildInputs = [
      gzip
      pkg-config
      installShellFiles

      # Use nixpkgs zlib-ng once rust-lang/libz-sys#206 merges
      cmake
    ];

    buildInputs = let
      d = darwin.apple_sdk.frameworks;
    in
      [
        libgit2
        libssh2
      ]
      ++ lib.optionals (!stdenv.hostPlatform.isDarwin) [openssl]
      ++ lib.optionals stdenv.isDarwin [
        d.Security
        d.SystemConfiguration
        libiconv
      ];

    nativeCheckInputs = [
      # For signing test
      gnupg
      openssh

      # For git subprocess test
      git
    ];

    env = {
      NIX_JJ_GIT_HASH = gitRev;
      RUST_BACKTRACE = 1;
      # Use nixpkgs libs rather than the vendored libs
      LIBGIT2_NO_VENDOR = 1;
      LIBSSH2_SYS_USE_PKG_CONFIG = 1;
    };

    postInstall = let
      jj = "$out/bin/jj";
    in
      lib.optionalString (stdenv.buildPlatform.canExecute stdenv.hostPlatform) ''
        ${jj} util install-man-pages man
        installManPage ./man/man1/*

        installShellCompletion --cmd jj \
          --bash <(${jj} util completion bash) \
          --fish <(${jj} util completion fish) \
          --zsh <(${jj} util completion zsh)
      '';

    meta = {
      description = "Git-compatible DVCS that is both simple and powerful";
      homepage = "https://github.com/jj-vcs/jj";
      license = lib.licenses.asl20;
      mainProgram = "jj";
    };
  })
