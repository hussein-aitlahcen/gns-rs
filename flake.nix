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
      in rec {
        devShell = mkShell {
          buildInputs = [ rust-nightly clang protobuf openssl pkg-config  ];
          LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
          LD_LIBRARY_PATH =
            lib.makeLibraryPath [ clangStdenv.cc.cc.lib openssl protobuf ];
          CPLUS_INCLUDE_PATH = "${openssl.dev}/include";
        };
      });
}
