# Rust abstraction for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets)

[![Crates.io](https://img.shields.io/crates/v/game-networking-sockets.svg)](https://crates.io/crates/game-networking-sockets)
[![Docs](https://docs.rs/game-networking-sockets/badge.svg)](https://docs.rs/game-networking-sockets)

A simple, high-level, type-safe wrapper for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets).

- [**Read the reliable chat client and server example**](./example/src/main.rs)

Your application does not need to run under Steam. This wrapper targets the open-source version of the library only.

Some features are still missing. If you want to add more, open an issue or a pull request.

This repository contains two crates:

- `gns-sys` builds Valve's C++ library and generates the bindings for it. Cargo
  compiles the C++ library for you, so you do not need to install it first.
- `gns` is the high-level, type-safe Rust wrapper.

## Building

Building `gns-sys` compiles the C++ library, which needs a few system libraries and tools.

You need these system libraries:
- `clang`
- `protobuf`
- `openssl`
- `abseil`, if you use a recent version of protobuf

You need these tools in your `$PATH`:
- `git`
- `protobuf-compiler`

### Windows

On Windows the build uses [vcpkg](https://github.com/microsoft/vcpkg) in manifest mode to gather and
build the dependencies. You only need `clang` installed and `git` available in your `$PATH`.

### macOS

#### Apple Silicon

- Install these dependencies:
```bash
brew install openssl@3 protobuf@21
```

- Check that you are using Protobuf 21.x:
```bash
protoc --version   # should print 3.21.x
```

- An error such as `no member named 'c_str' in 'std::string_view'` means the build picked up a newer
  Protobuf. Either unlink the newer version or point CMake at 21.x:
```bash
# Only if needed
brew unlink protobuf
brew link --overwrite protobuf@21   # add --force if keg-only warns
```

#### Intel

Not tested on Intel.
