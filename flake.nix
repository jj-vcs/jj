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

      # When we're running in the shell, we want to use rustc with a bunch
      # of extra junk to ensure that rust-analyzer works, clippy etc are all
      # installed.
      rustShellToolchain = (pkgs.rust-bin.selectLatestNightlyWith (t: t.default)).override {
        # NOTE (aseipp): explicitly add rust-src to the rustc compiler only in
        # devShell. this in turn causes a dependency on the rust compiler src,
        # which bloats the closure size by several GiB. but doing this here and
        # not by default avoids the default flake install from including that
        # dependency, so it's worth it
        #
        # relevant PR: https://github.com/rust-lang/rust/pull/129687
        extensions = ["rust-src" "rust-analyzer"];
      };

      # But, whenever we are running CI builds or checks, we want to use a
      # smaller closure. This reduces the CI impact on fresh clones/VMs, etc.
      rustMinimalPlatform =
        let platform = pkgs.rust-bin.selectLatestNightlyWith (t: t.minimal);
        in pkgs.makeRustPlatform { rustc = platform; cargo = platform; };

      nativeBuildInputs = with pkgs;
        [ ]
        ++ lib.optionals stdenv.isLinux [
          mold-wrapped
        ];

      buildInputs = with pkgs;
        [ ]
        ++ lib.optionals stdenv.isDarwin [
          darwin.apple_sdk.frameworks.Security
          darwin.apple_sdk.frameworks.SystemConfiguration
          libiconv
        ];

      nativeCheckInputs = with pkgs; [
        # for signing tests
        gnupg
        openssh

        # for git subprocess test
        git

        # for schema tests
        taplo
      ];

      env = {
        RUST_BACKTRACE = 1;
        CARGO_INCREMENTAL = "0"; # https://github.com/rust-lang/rust/issues/139110
      };
    in {
      formatter = pkgs.alejandra;

      packages = {
        jujutsu = rustMinimalPlatform.buildRustPackage {
          pname = "jujutsu";
          version = "unstable-${self.shortRev or "dirty"}";

          cargoBuildFlags = ["--bin" "jj"]; # don't build and install the fake editors
          useNextest = true;
          cargoTestFlags = ["--profile" "ci"];
          src = filterSrc ./. [
            ".*\\.nix$"
            "^.jj/"
            "^flake\\.lock$"
            "^target/"
          ];

          # Taplo requires SystemConfiguration access, as it unconditionally creates a
          # reqwest client.
          sandboxProfile = ''
            (allow mach-lookup (global-name "com.apple.SystemConfiguration.configd"))
          '';

          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = nativeBuildInputs ++ [pkgs.installShellFiles];
          inherit buildInputs nativeCheckInputs;

          env =
            env
            // {
              RUSTFLAGS = pkgs.lib.optionalString pkgs.stdenv.isLinux "-C link-arg=-fuse-ld=mold";
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

          meta = {
            description = "Git-compatible DVCS that is both simple and powerful";
            homepage = "https://github.com/jj-vcs/jj";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "jj";
          };
        };
        default = self.packages.${system}.jujutsu;
      };

      checks.jujutsu = self.packages.${system}.jujutsu.overrideAttrs ({...}: {
        # The default Rust infrastructure runs all builds in the release
        # profile, which is significantly slower. Run this under the `test`
        # profile instead, which matches all our other CI systems, Cargo, etc.
        cargoBuildType = "test";
        cargoCheckType = "test";

        # By default, `flake check` will want to run the install phase, but
        # because we override the cargoBuildType, it fails to find the proper
        # binary. But we don't even care about the binary or even the buildPhase
        # in this case; just remove them both.
        buildPhase = "true";
        installPhase = "touch $out";
      });

      devShells.default = let
        packages = with pkgs; [
          rustShellToolchain

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
          # nixos does not work with uv-installed python
          python3
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
