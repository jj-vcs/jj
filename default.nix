{
  #
  # Stdenv
  #
  lib,
  stdenv,
  rustPlatform,
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
  filterSrc = src: regexes:
    lib.cleanSourceWith {
      inherit src;
      filter = path: type: let
        relPath = lib.removePrefix (toString src + "/") (toString path);
      in
        lib.all (re: builtins.match re relPath == null) regexes;
    };   
in
  rustPlatform.buildRustPackage (finalAttrs: {
    strictDeps = true;

    pname = "jujutsu";
    version = "unstable-" + gitRev;

    cargoLock.lockFile = ./Cargo.lock;

    cargoBuildFlags = ["--bin" "jj"];
    buildFeatures = ["packaging"];
    buildType = "release";

    doCheck = false;
    useNextest = true;

    src = filterSrc ./. [
      ".*\\.nix$"
      "^.jj/"
      "^flake\\.lock$"
      "^target/"
    ];

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
      lib.optionalString ''
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
