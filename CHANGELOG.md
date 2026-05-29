# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Because the crate is pre-1.0, a **minor** version bump signals breaking changes.

## [0.2.0] - 2026-05-29

The **v2** line of `gns-rs`: a near-total rewrite of the safe wrapper for
soundness, a unified error type, and a more idiomatic, iterator-based API,
on top of an upgraded GameNetworkingSockets. **Almost everything below is a
breaking change** — code written against `0.1.x` will need updating.

### Breaking changes

#### Errors

- `GnsResult<T>` is now `Result<T, GnsError>` (was `Result<T, EResult>`).
- `GnsError` is now a rich enum implementing `std::error::Error` (via
  `thiserror`), with variants `Init`, `Listen`, `Connect`, `Receive`,
  `Accept`, `Close`, `Api(EResult)`, and `Config(&'static str)`. It was
  previously a newtype `GnsError(EResult)` with `into_result()` / `From`
  conversions (both removed).
- `GnsGlobal::get()` now returns `GnsResult<&'static GnsGlobal>` (was
  `Result<Arc<GnsGlobal>, String>`).
- `listen` / `connect` now return `GnsResult<…>` (was `Result<…, ()>`).
- `set_global_config_value` / `set_connection_config_value` now return
  `GnsResult<()>` (was `Result<(), ()>`).

#### Global & socket lifetime

- `GnsGlobal` is now a process-wide singleton accessed by `&'static`
  reference (backed by `OnceLock`). `GnsSocket::new` takes
  `&'static GnsGlobal` instead of `Arc<GnsGlobal>`; the `Arc` cloning is
  gone.
- `GnsListenSocket` and `GnsPollGroup` are now `pub(crate)` (were `pub`) —
  they are internal implementation details.

#### Receiving — now iterator-based

- `poll_messages::<K>(callback) -> Option<usize>` is **removed**, replaced by:
  - `receive_messages::<K>() -> GnsResult<ReceivedMessages<K>>` — owning
    iterator over an inline `K`-slot buffer (no heap allocation); yields each
    `GnsNetworkMessage<ToReceive>` **by value** so it can be kept or dropped.
  - `receive_messages_into(&mut [MessageSlot]) -> GnsResult<ReceivedMessagesInto<'_>>`
    — zero-move variant that borrows a caller-owned buffer (reuse one buffer
    across a poll loop for zero per-call cost).
- `poll_event::<K>(callback) -> usize` is **removed**, replaced by
  `receive_events() -> impl Iterator<Item = GnsConnectionEvent>`, which drains
  the pending events lazily. Bound the work per tick with `.take(n)`.
- New public types backing these: `ReceivedMessages<K>`,
  `ReceivedMessagesInto<'a>`, and the `MessageSlot` buffer-cell alias.
- The sealed `IsReady::receive` primitive is now slice-based and returns
  `GnsResult<usize>` (was a const-generic array returning a `usize::MAX`
  sentinel). `IsReady` is now a sealed trait.

#### Sending & messages

- Outbound payloads are now owned via the new `unsafe trait Payload`
  (implemented for `Box<[u8]>`, `Vec<u8>`, `String`, `Arc<[u8]>`,
  `&'static [u8]`, `&'static str`). `GnsUtils::allocate_message` is now
  `allocate_message<P: Payload>(conn, flags, payload)` and **takes ownership**
  of the payload (was `allocate_message(conn, flags: i32, payload: &[u8])`,
  which copied). Zero-copy for already-owned heap buffers; GNS frees the
  payload through its `Drop`.
- `send_messages` now accepts `impl IntoIterator<Item = GnsNetworkMessage<ToSend>>`
  (was `Vec<…>`) and returns `Vec<SendOutcome>` (was
  `Vec<Either<GnsMessageNumber, EResult>>`). The new `SendOutcome`
  (`Sent` / `Failed` / `Skipped`) models GNS's batched-failure semantics
  soundly and hands ownership of failed/skipped messages back to the caller.
- New `send_message(msg) -> GnsResult<GnsMessageNumber>` convenience for the
  single-message case.
- The `MayDrop` trait and `GnsNetworkMessage::set_payload` are **removed**.
  `GnsNetworkMessage<T>` no longer carries a `T: MayDrop` bound.

#### Flags & lanes

- Send flags are now the type-safe `SendFlags` bitflags struct (was raw
  `i32`). `GnsNetworkMessage::flags()` / `set_flags()` and
  `allocate_message` use `SendFlags`.
- `GnsLane` is now a struct `{ priority: i32, weight: u16 }` with
  `GnsLane::new(..)` (was the tuple alias `(Priority, Weight)`; the `Priority`
  and `Weight` aliases are removed).

#### Configuration

- `GnsConfig::Int32` now holds `i32` (was `u32`).

#### Debug output

- `enable_debug_output` now accepts a **capturing** closure,
  `impl Fn(ESteamNetworkingSocketsDebugOutputType, &str) + Send + Sync + 'static`,
  and borrows the message as `&str`. Previously it took a bare
  `fn(_, msg: String)` pointer that allocated a `String` per message and could
  not capture state. The registration is stored in an `OnceLock` (was a
  `static mut`).

#### Connection helpers

- `close_connection` now returns `GnsResult<()>` (was `bool`) and takes the
  diagnostic string as `debug: Option<&CStr>` (was `debug: &str`, which
  allocated a `CString` and panicked on an interior NUL); pass `None` to send
  no string and avoid all allocation.
- `accept` now returns `Err(GnsError::Accept)` on poll-group assignment
  failure instead of `panic!`-ing.

### Added

- `GnsConfig::CStr(&CStr)` — zero-allocation string config variant (skips the
  `CString` allocation the `String` variant performs).
- `GnsConnection::from_raw(handle)` and `GnsConnection::is_valid()`.
- `GnsGlobal::queue_count()` — introspection on the live event-queue count.

### Fixed / soundness

- `send_messages` is now sound: failed and skipped messages are returned to
  the caller (via `SendOutcome`) instead of being leaked or double-freed,
  matching GNS's batched-failure contract.
- Outbound message payloads are owned for their whole lifetime and freed
  exactly once through `Payload`/`Drop`, removing the previous reliance on
  borrowed `&[u8]` buffers outliving the asynchronous send.

### Internal / build

- Upgraded the bundled GameNetworkingSockets submodule
  (`v1.4.1-191-g35d7b80` → `v1.4.1-303-g1cd8e79`).
- Updated Windows MSVC linking in `gns-sys/build.rs`.
- `gns-sys` bumped `0.1.5` → `0.2.0`.

[0.2.0]: https://github.com/hussein-aitlahcen/gns-rs/compare/1aea9b1...gns-v2
