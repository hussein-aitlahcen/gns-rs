{
  description = "Valve GameNetworkingSockets Wrapper";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/release-26.05";
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
      in
      let
        rust-nightly = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
          targets = [ "x86_64-unknown-linux-musl" ];
        };
      in rec {
        devShell = pkgs.mkShell {
          buildInputs = [
            rust-nightly
            pkgs.cmake
            pkgs.clang
            pkgs.pkgsStatic.openssl
            pkgs.pkgsStatic.protobuf
            pkgs.pkgsStatic.abseil-cpp
            pkgs.pkg-config
          ];
          PROTOC = "${pkgs.pkgsStatic.protobuf}/bin/protoc";
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          RUSTFLAGS = "-L${pkgs.pkgsStatic.openssl.out}/lib -L${pkgs.pkgsStatic.protobuf}/lib -L${pkgs.pkgsStatic.abseil-cpp}/lib";
        };
      });
}
