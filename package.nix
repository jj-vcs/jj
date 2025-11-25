{
  lib,
  rust-bin,
  makeRustPlatform,
  stdenv,
  mold-wrapped,
  gnupg,
  openssh,
  git,
  installShellFiles,
  self,
}: let
  packageVersion = (lib.importTOML ./Cargo.toml).workspace.package.version;

  # But, whenever we are running CI builds or checks, we want to use a
  # smaller closure. This reduces the CI impact on fresh clones/VMs, etc.
  rustMinimalPlatform = let
    platform = rust-bin.selectLatestNightlyWith (t: t.minimal);
  in
    makeRustPlatform {
      rustc = platform;
      cargo = platform;
    };

  nativeBuildInputs = lib.optionals stdenv.isLinux [
    mold-wrapped
  ];

  buildInputs = [];

  nativeCheckInputs = [
    # for signing tests
    gnupg
    openssh

    # for git subprocess test
    git
  ];

  env = {
    RUST_BACKTRACE = 1;
    CARGO_INCREMENTAL = "0"; # https://github.com/rust-lang/rust/issues/139110
  };
in
  rustMinimalPlatform.buildRustPackage {
    pname = "jujutsu";
    version = "${packageVersion}-unstable-${self.shortRev or self.dirtyShortRev or "dirty"}";

    cargoBuildFlags = [
      "--bin"
      "jj"
    ]; # don't build and install the fake editors
    useNextest = true;
    cargoTestFlags = [
      "--profile"
      "ci"
    ];
    src = lib.fileset.toSource {
      root = ./.;
      fileset =
        lib.fileset.fileFilter (
          {
            name,
            hasExt,
            ...
          }:
            !hasExt "nix"
            && !lib.elem name [
              "target"
              ".jj"
              "flake.lock"
            ]
        )
        ./.;
    };

    cargoLock.lockFile = ./Cargo.lock;
    nativeBuildInputs = nativeBuildInputs ++ [installShellFiles];
    inherit buildInputs nativeCheckInputs;

    env =
      env
      // {
        RUSTFLAGS = lib.optionalString stdenv.isLinux "-C link-arg=-fuse-ld=mold";
        NIX_JJ_GIT_HASH = self.rev or "";
      };

    postInstall = ''
      $out/bin/jj util install-man-pages man
      installManPage ./man/man1/*

      installShellCompletion --cmd jj \
        --bash <(COMPLETE=bash $out/bin/jj) \
        --fish <(COMPLETE=fish $out/bin/jj) \
        --zsh <(COMPLETE=zsh $out/bin/jj)
    '';

    passthru = {
      inherit
        env
        nativeBuildInputs
        buildInputs
        nativeCheckInputs
        ;
    };

    meta = {
      description = "Git-compatible DVCS that is both simple and powerful";
      homepage = "https://github.com/jj-vcs/jj";
      license = lib.licenses.asl20;
      mainProgram = "jj";
    };
  }
