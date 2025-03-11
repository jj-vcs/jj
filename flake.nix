{
  description = "Jujutsu VCS, a Git-compatible DVCS that is both simple and powerful";

  inputs = {
    # For listing and iterating nix systems
    flake-utils.url = "github:numtide/flake-utils";

    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    # For installing non-standard rustc versions
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
  }:
    {
      overlays.default = final: prev: {
        jujutsu = self.packages.${final.system}.jujutsu;
      };
    }
    // (flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };

      minimalPlatform = let
        platform = pkgs.rust-bin.selectLatestNightlyWith (t: t.minimal);
      in
        pkgs.makeRustPlatform {
          rustc = platform;
          cargo = platform;
        };

      gitRev = self.rev or self.dirtyRev or "dirty";

      jujutsu = pkgs.callPackage ./. {
        inherit gitRev;
        rustPlatform = minimalPlatform;
      };
    in {
      formatter = pkgs.alejandra;

      packages = {
        inherit jujutsu;
        default = self.packages.${system}.jujutsu;
      };

      checks.jujutsu = self.packages.${system}.jujutsu.overrideAttrs {
        doCheck = true;
        cargoBuildType = "ci";
        cargoBuildFeatures = ["test-fakes"];
        buildPhase = "true";
        installPhase = "touch $out";
      };

      devShells.default = let
        rustShellToolchain = pkgs.rust-bin.selectLatestNightlyWith (t:
          t.default.override {
            extensions = ["rust-src" "rust-analyzer"];
          });

        packages = let
          p = pkgs;
        in [
          rustShellToolchain

          # Additional tools recommended by contributing.md
          p.bacon
          p.cargo-deny
          p.cargo-insta
          p.cargo-nextest

          # Miscellaneous tools
          p.watchman

          # In case you need to run `cargo run --bin gen-protos`
          p.protobuf

          # For building the documentation website
          p.uv
          # nixos does not work with uv-installed python
          p.python3
        ];

        # on macOS and Linux, use faster parallel linkers that are much more
        # efficient than the defaults. these noticeably improve link time even for
        # medium sized rust projects like jj
        rustLinkerFlags = let
          inherit (pkgs.lib) optionals;
          std = pkgs.stdenv;
        in
          optionals std.isLinux [
            "-Wl,--compress-debug-sections=zstd"
          ]
          ++ optionals std.isDarwin [
            # on darwin, /usr/bin/ld actually looks at the environment variable
            # $DEVELOPER_DIR, which is set by the nix stdenv, and if set,
            # automatically uses it to route the `ld` invocation to the binary
            # within. in the devShell though, that isn't what we want; it's
            # functional, but Xcode's linker as of ~v15 (not yet open source)
            # is ultra-fast and very shiny; it is enabled via -ld_new, and on by
            # default as of v16+
            "--ld-path=$(unset DEVELOPER_DIR; /usr/bin/xcrun --find ld)"
            "-ld_new"
          ];

        rustLinkFlagsString =
          pkgs.lib.concatStringsSep " "
          (pkgs.lib.concatMap (x: ["-C" "link-arg=${x}"]) rustLinkerFlags);
      in
        pkgs.mkShell.override {
          stdenv =
            if pkgs.stdenv.hostPlatform.isElf
            then pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv
            else pkgs.stdenv;
        } {
          inherit packages;
          name = "jujutsu";
          inputsFrom = [self.checks.${system}.jujutsu];
          env = {
            LIBGIT2_NO_VENDOR = 1;
            LIBSSH_SYS_USE_PKG_CONFIG = 1;
            RUST_BACKTRACE = 1;
          };
          # The `RUSTFLAGS` environment variable is set in `shellHook` instead of `env`
          # to allow the `xcrun` command above to be interpreted by the shell.
          shellHook = ''
            export RUSTFLAGS="-Zthreads=0 ${rustLinkFlagsString}"
          '';
        };
    }));
}
