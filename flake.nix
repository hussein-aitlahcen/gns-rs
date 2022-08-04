{
  description = "Cosmwasm VM";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      in with pkgs;
      let
        # Nightly rust used for wasm runtime compilation
        rust-nightly = rust-bin.nightly.latest.default;

        # Crane lib instantiated with current nixpkgs
        crane-lib = crane.mkLib pkgs;

        # Crane pinned to nightly Rust
        crane-nightly = crane-lib.overrideToolchain rust-nightly;

        # Default args to crane
        common-args = {
          src = lib.cleanSourceWith {
            filter = lib.cleanSourceFilter;
            src = lib.cleanSourceWith {
              filter = nix-gitignore.gitignoreFilterPure (name: type: true)
                [ ./.gitignore ] ./.;
              src = ./.;
            };
          };
        };

        # Common dependencies used for caching
        common-deps = crane-nightly.buildDepsOnly common-args;

        common-cached-args = common-args // { cargoArtifacts = common-deps; };

      in rec {
        packages.pkg = crane-nightly.buildPackage common-cached-args;
        packages.default = packages.pkg;
        checks = {
          package = packages.default;
          clippy = crane-nightly.cargoClippy (common-cached-args // {
            cargoClippyExtraArgs = "-- --deny warnings";
          });
          fmt = crane-nightly.cargoFmt common-args;
        };
        devShell = mkShell {
          buildInputs = [ rust-nightly clang protobuf openssl pkg-config];
          LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
          LD_LIBRARY_PATH = lib.makeLibraryPath [ stdenv.cc.cc.lib openssl protobuf ];
          CPLUS_INCLUDE_PATH = "${openssl.dev}/include";
        };
      });
}
