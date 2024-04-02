# Rust abstraction for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets)

Simple, high-level and (somehow) type-safe wrapper for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets).

- [**Go ahead and read the documentation**](https://hussein-aitlahcen.github.io/gns-rs/gns/)
- [**Have a quick look at the reliable chat client/server implementation**](./example/src/main.rs)

The library does not require your application to be running with Steam and this wrapper is intended to wrap the open-source version only.

Some features might be missing, if you are interested to introduce more abstraction, feel free to open a PR/Issue.

Libraries:
- `gns-sys` is the C++ library from Valve compiled with bindings generated (the library is directly compiled by cargo so you don't need to have it already installed).
- `gns` is the high level, type-safe Rust wrapper.

System libraries required:
- `clang`
- `protobuf`
- `openssl`
- `abseil`
