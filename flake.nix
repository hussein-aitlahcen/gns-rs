{
  description = "Valve GameNetworkingSockets Wrapper";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
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
          buildInputs = [ cmake rust-nightly clang_15 openssl protobuf abseil-cpp_202401 pkg-config  ];
          PROTOC = "${protobuf}/bin/protoc";
          LIBCLANG_PATH = "${llvmPackages_15.libclang.lib}/lib";
          LD_LIBRARY_PATH =
            lib.makeLibraryPath [ clang15Stdenv.cc.cc.lib openssl protobuf abseil-cpp_202401 ];
          CPLUS_INCLUDE_PATH = "${openssl.dev}/include";
        };
      });
}
