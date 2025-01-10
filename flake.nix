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
        overlays = [
          rust-overlay.overlays.default
        ];
      };

      filterSrc = src: regexes:
        pkgs.lib.cleanSourceWith {
          inherit src;
          filter = path: type: let
            relPath = pkgs.lib.removePrefix (toString src + "/") (toString path);
          in
            pkgs.lib.all (re: builtins.match re relPath == null) regexes;
        };

      ourRustVersion = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default);

      ourRustPlatform = pkgs.makeRustPlatform {
        rustc = ourRustVersion;
        cargo = ourRustVersion;
      };

      nativeBuildInputs = with pkgs;
        [
          gzip
          pkg-config

          # for libz-ng-sys (zlib-ng)
          # TODO: switch to the packaged zlib-ng and drop this dependency
          cmake
        ]
        ++ lib.optionals stdenv.isLinux [
          mold-wrapped
        ];

      buildInputs = with pkgs;
        [
          openssl
          libgit2
          libssh2
        ]
        ++ lib.optionals stdenv.isDarwin [
          darwin.apple_sdk.frameworks.Security
          darwin.apple_sdk.frameworks.SystemConfiguration
          libiconv
        ];

      nativeCheckInputs = with pkgs; [
        # for signing tests
        gnupg
        openssh
      ];

      env = {
        LIBSSH2_SYS_USE_PKG_CONFIG = "1";
        RUST_BACKTRACE = 1;
      };
    in {
      formatter = pkgs.alejandra;
      checks.jujutsu = self.packages.${system}.jujutsu;

      packages = {
        jujutsu = ourRustPlatform.buildRustPackage {
          pname = "jujutsu";
          version = "unstable-${self.shortRev or "dirty"}";

          buildFeatures = ["packaging"];
          cargoBuildFlags = ["--bin" "jj"]; # don't build and install the fake editors
          useNextest = true;
          src = filterSrc ./. [
            ".*\\.nix$"
            "^.jj/"
            "^flake\\.lock$"
            "^target/"
          ];

          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = nativeBuildInputs ++ [pkgs.installShellFiles];
          inherit buildInputs nativeCheckInputs;

          env =
            env
            // {
              RUSTFLAGS = pkgs.lib.optionalString pkgs.stdenv.isLinux "-C link-arg=-fuse-ld=mold";
              NIX_JJ_GIT_HASH = self.rev or "";
              CARGO_INCREMENTAL = "0";
            };

          postInstall = ''
            $out/bin/jj util mangen > ./jj.1
            installManPage ./jj.1

            installShellCompletion --cmd jj \
              --bash <(COMPLETE=bash $out/bin/jj) \
              --fish <(COMPLETE=fish $out/bin/jj) \
              --zsh <(COMPLETE=zsh $out/bin/jj)
          '';

          meta = {
            description = "Git-compatible DVCS that is both simple and powerful";
            homepage = "https://github.com/jj-vcs/jj";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "jj";
          };
        };
        default = self.packages.${system}.jujutsu;
      };

      devShells.default = let
        packages = with pkgs; [
          # NOTE (aseipp): explicitly add rust-src to the rustc compiler only in
          # devShell. this in turn causes a dependency on the rust compiler src,
          # which bloats the closure size by several GiB. but doing this here
          # and not by default avoids the default flake install from including
          # that dependency, so it's worth it
          #
          # relevant PR: https://github.com/rust-lang/rust/pull/129687
          (ourRustVersion.override {
            extensions = ["rust-src" "rust-analyzer"];
          })

          # Additional tools recommended by contributing.md
          bacon
          cargo-deny
          cargo-insta
          cargo-nextest

          # Miscellaneous tools
          watchman

          # In case you need to run `cargo run --bin gen-protos`
          protobuf

          # For building the documentation website
          uv
        ];

        # on macOS and Linux, use faster parallel linkers that are much more
        # efficient than the defaults. these noticeably improve link time even for
        # medium sized rust projects like jj
        rustLinkerFlags =
          if pkgs.stdenv.isLinux
          then ["-fuse-ld=mold" "-Wl,--compress-debug-sections=zstd"]
          else if pkgs.stdenv.isDarwin
          then
            # on darwin, /usr/bin/ld actually looks at the environment variable
            # $DEVELOPER_DIR, which is set by the nix stdenv, and if set,
            # automatically uses it to route the `ld` invocation to the binary
            # within. in the devShell though, that isn't what we want; it's
            # functional, but Xcode's linker as of ~v15 (not yet open source)
            # is ultra-fast and very shiny; it is enabled via -ld_new, and on by
            # default as of v16+
            ["--ld-path=$(unset DEVELOPER_DIR; /usr/bin/xcrun --find ld)" "-ld_new"]
          else [];

        rustLinkFlagsString =
          pkgs.lib.concatStringsSep " "
          (pkgs.lib.concatMap (x: ["-C" "link-arg=${x}"]) rustLinkerFlags);

        # The `RUSTFLAGS` environment variable is set in `shellHook` instead of `env`
        # to allow the `xcrun` command above to be interpreted by the shell.
        shellHook = ''
          export RUSTFLAGS="-Zthreads=0 ${rustLinkFlagsString}"
        '';
      in
        pkgs.mkShell {
          name = "jujutsu";
          packages = packages ++ nativeBuildInputs ++ buildInputs ++ nativeCheckInputs;
          inherit env shellHook;
        };
    }));
}
