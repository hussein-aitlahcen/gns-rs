# Rust abstraction for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets)

Simple, high-level and (somehow) type-safe wrapper for [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets).

Some features might be missing, if you are interested to introduce more abstraction, feel free to open a PR/Issue.

Libraries:
- `gns-sys` is C++ library from Valve compiled with bindings generated.
- `gns` is the high level Rust wrapper.

Even if `gns` is a high level abstraction, it reexport most of the low level functions/structures/constants.

System libraries required:
- `protobuf`
- `openssl`

[Have a look at the reliable chat client/server implementation](./example/src/main.rs)
