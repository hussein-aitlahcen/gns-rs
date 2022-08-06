# Rust abstraction for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets)

Simple, high-level and (somehow) type-safe wrapper for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets).

The library does not require your application to be running with Steam and this wrapper is intended to wrap the open-source version only.

Some features might be missing, if you are interested to introduce more abstraction, feel free to open a PR/Issue.

Libraries:
- `gns-sys` is the C++ library from Valve compiled with bindings generated (the library is directly compiled by cargo so you don't need to have it already installed).
- `gns` is the high level, type-safe Rust wrapper.

Even if `gns` is a high level abstraction, it re-export most of the low level functions/structures/constants.

System libraries required:
- `protobuf`
- `openssl`

[Have a look at the reliable chat client/server implementation](./example/src/main.rs)
