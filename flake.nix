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
      overlays.default = final: prev:
        rust-overlay.overlays.default final prev
        // {
          jujutsu = final.callPackage ./package.nix {inherit self;};
        };
    }
    // (flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [
          self.overlays.default
        ];
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
    in {
      formatter = pkgs.alejandra;

      packages = {
        inherit (pkgs) jujutsu;
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
          self.formatter.${system}

          # In case you need to run `cargo run --bin gen-protos`
          protobuf

          # For building the documentation website
          uv
          # nixos does not work with uv-installed python
          python3
          python3Packages.numpy
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

        inherit (self.packages.${system}.jujutsu.passthru) env nativeBuildInputs buildInputs nativeCheckInputs;
      in
        pkgs.mkShell {
          name = "jujutsu";
          packages = packages ++ nativeBuildInputs ++ buildInputs ++ nativeCheckInputs;
          inherit env shellHook;
        };
    }));
}
