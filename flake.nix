{
  description = "Jujutsu VCS, a Git-compatible DVCS that is both simple and powerful";

  inputs = {
    # For listing and iterating nix systems
    flake-utils.url = "github:numtide/flake-utils";

    # For installing non-standard rustc versions
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    let
      systems = with flake-utils.lib.system; [
        aarch64-linux
        aarch64-darwin
        i686-linux
        x86_64-darwin
        x86_64-linux
      ];
    in
    {
      overlays.default = (final: prev: {
        jujutsu = self.packages.${final.system}.jujutsu;
      });
    } //
    (flake-utils.lib.eachSystem systems (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            rust-overlay.overlays.default
          ];
        };
      in
      {
        packages = {
          jujutsu = pkgs.rustPlatform.buildRustPackage rec {
            pname = "jujutsu";
            version = "unstable-${self.shortRev or "dirty"}";
            buildNoDefaultFeatures = true;
            buildFeatures = [ "jujutsu-lib/legacy-thrift" ];
            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = with pkgs; [
              gzip
              installShellFiles
              makeWrapper
              pkg-config
            ];
            buildInputs = with pkgs; [ openssl dbus sqlite ]
            ++ lib.optionals stdenv.isDarwin [
              darwin.apple_sdk.frameworks.Security
              darwin.apple_sdk.frameworks.SystemConfiguration
              libiconv
            ];
            postInstall = ''
              $out/bin/jj debug mangen > ./jj.1
              installManPage ./jj.1

              $out/bin/jj debug completion --bash > ./completions.bash
              installShellCompletion --bash --name ${pname}.bash ./completions.bash
              $out/bin/jj debug completion --fish > ./completions.fish
              installShellCompletion --fish --name ${pname}.fish ./completions.fish
              $out/bin/jj debug completion --zsh > ./completions.zsh
              installShellCompletion --zsh --name _${pname} ./completions.zsh
            '';
          };
          default = self.packages.${system}.jujutsu;
        };
        apps.default = {
          type = "app";
          program = "${self.packages.${system}.jujutsu}/bin/jj";
        };
        checks.jujutsu = self.packages.${system}.jujutsu.overrideAttrs ({ ... }: {
          cargoBuildType = "debug";
          cargoCheckType = "debug";
          preCheck = ''
            export RUST_BACKTRACE=1
          '';
        });
        formatter = pkgs.nixpkgs-fmt;
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            # Using the minimal profile with explicit "clippy" extension to avoid
            # two versions of rustfmt
            (rust-bin.stable."1.61.0".minimal.override {
              extensions = [
                "rust-src" # for rust-analyzer
                "clippy"
              ];
            })

            # The CI checks against the latest nightly rustfmt, so we should too.
            (rust-bin.selectLatestNightlyWith (toolchain: toolchain.rustfmt))

            # Required build dependencies
            openssl
            pkg-config # to find openssl

            # Additional tools recommended by contributing.md
            cargo-deny
            cargo-insta
            cargo-nextest
            cargo-watch
          ];
        };
      }));
}
