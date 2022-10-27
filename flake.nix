{
  description = "Valve GameNetworkingSockets Wrapper";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      in with pkgs;
      let
        rust-nightly = rust-bin.nightly.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in rec {
        devShell = mkShell {
          buildInputs = [ rust-nightly clang protobuf openssl pkg-config  ];
          PROTOC = "${protobuf}/bin/protoc";
          LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
          LD_LIBRARY_PATH =
            lib.makeLibraryPath [ clangStdenv.cc.cc.lib openssl protobuf ];
          CPLUS_INCLUDE_PATH = "${openssl.dev}/include";
        };
      });
}
