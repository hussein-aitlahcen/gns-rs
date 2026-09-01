//! # Rust wrapper for Valve GameNetworkingSockets
//!
//! This crate wraps the low-level GameNetworkingSockets library and gives you
//! two things:
//!
//! - **Type safety.** The socket type records its own state, so the compiler
//!   rejects any operation that the current state does not allow. Every public
//!   operation is safe to call.
//! - **A high-level API.** You never write FFI code. The API is plain,
//!   idiomatic Rust.
//!
//! # Example
//!
//! ```
//! use gns::{GnsGlobal, GnsSocket, IsCreated};
//! use std::net::Ipv6Addr;
//! use std::time::Duration;
//!
//! // Do not use `unwrap` in production. This example uses it to keep the
//! // interesting calls easy to read.
//!
//! // Initialize the global networking state. A process has exactly one.
//! let gns_global = GnsGlobal::get().unwrap();
//!
//! // Create a socket. The type parameter records the socket state, and
//! // `GnsSocket::new` is only available in the initial `IsCreated` state.
//! let gns_socket = GnsSocket::<IsCreated>::new(gns_global);
//!
//! // Choose your own port.
//! let port = 9001;
//!
//! // `connect` moves the socket from `IsCreated` to `IsClient`, which gives
//! // you the client operations.
//! let client = gns_socket.connect(Ipv6Addr::LOCALHOST.into(), port).unwrap();
//!
//! // A connected socket needs three calls in your main loop:
//! //
//! // 1. Poll for new messages.
//! // 2. Poll for connection status changes.
//! // 3. Poll for the low-level callbacks that the underlying library needs.
//! //
//! // Clients and servers use the same three calls. Only the scope differs. On
//! // a client they cover the single connection. On a server they cover every
//! // connected client.
//!
//! // Run the low-level callbacks.
//! gns_global.poll_callbacks();
//!
//! // Receive at most 100 messages and print each payload.
//! for message in client.receive_messages::<100>().expect("failed to recv").into_iter() {
//!   println!("{}", core::str::from_utf8(message.payload()).unwrap());
//! }
//!
//! // This example ignores events. A real program reads them to react when the
//! // connection opens or closes.
//! for _event in client.receive_events() {
//! }
//!
//! // Wait before the next iteration.
//! std::thread::sleep(Duration::from_millis(10))
//! ```
//!
//! # How events reach a socket
//!
//! Each [`GnsSocket`] registers a weak reference to its event queue with
//! [`GnsGlobal`]. When GameNetworkingSockets reports a connection-state change,
//! the callback uses that registry to find the socket the event belongs to.
//! Dropping the socket removes its entry.

use crossbeam_queue::SegQueue;
pub use gns_sys as sys;
use std::sync::atomic::{AtomicI64, Ordering};
use std::{
    collections::HashMap,
    ffi::{c_void, CStr, CString},
    marker::PhantomData,
    mem::MaybeUninit,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex, OnceLock, RwLock, Weak},
    time::Duration,
};
use sys::*;

#[inline]
fn get_interface() -> *mut ISteamNetworkingSockets {
    unsafe { SteamAPI_SteamNetworkingSockets_v009() }
}

#[inline]
fn get_utils() -> *mut ISteamNetworkingUtils {
    unsafe { SteamAPI_SteamNetworkingUtils_v003() }
}

/// A network message number. This alias exists to make signatures readable.
pub type GnsMessageNumber = u64;

/// An error returned by the wrapper.
///
/// Most variants wrap the [`EResult`] that the underlying API returned. The
/// rest cover setup paths that report failure without an `EResult`.
///
/// The enum is non-exhaustive because wrapping more of the underlying API
/// adds variants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GnsError {
    #[error("GameNetworkingSockets_Init failed: {0}")]
    Init(String),
    #[error("listen failed: invalid handle")]
    Listen,
    #[error("connect failed: invalid handle")]
    Connect,
    #[error("create socket pair failed")]
    SocketPair,
    #[error("receive failed: invalid connection or poll group handle")]
    Receive,
    #[error("accept failed: could not set connection poll group")]
    Accept,
    #[error("close failed: invalid connection handle")]
    Close,
    #[error("steam api: {0:?}")]
    Api(EResult),
    #[error("config: {0}")]
    Config(&'static str),
}

pub type GnsResult<T> = Result<T, GnsError>;

/// Converts a `SteamNetworkingIPAddr` into a Rust [`IpAddr`], unmapping
/// IPv4-mapped IPv6 addresses back to plain IPv4.
#[inline]
fn ip_from_steam_ip_addr(addr: &SteamNetworkingIPAddr) -> IpAddr {
    let ipv4 = unsafe { addr.__bindgen_anon_1.m_ipv4 };
    if ipv4.m_8zeros == 0 && ipv4.m_0000 == 0 && ipv4.m_ffff == 0xffff {
        IpAddr::from(Ipv4Addr::from(ipv4.m_ip))
    } else {
        IpAddr::from(Ipv6Addr::from(unsafe { addr.__bindgen_anon_1.m_ipv6 }))
    }
}

/// Converts an `EResult` returned by an FFI call into a [`GnsResult`].
#[inline]
fn check(e: EResult) -> GnsResult<()> {
    match e {
        EResult::k_EResultOK => Ok(()),
        e => Err(GnsError::Api(e)),
    }
}

/// Owns the initialization and teardown of GameNetworkingSockets and its
/// singletons.
///
/// Call [`GnsGlobal::get()`] to obtain the instance. The first call initializes
/// GameNetworkingSockets, and later calls return the same instance.
pub struct GnsGlobal {
    utils: GnsUtils,
    next_queue_id: AtomicI64,
    /// Maps each socket to its event queue.
    ///
    /// Reads dominate: every connection-state callback from the
    /// GameNetworkingSockets service thread performs one lookup. Writes happen
    /// only when a socket is created or dropped, or in the rare case where a
    /// callback arrives for a socket that was dropped moments earlier. An
    /// `RwLock` lets those reads run concurrently.
    event_queues: RwLock<HashMap<i64, Weak<SegQueue<GnsConnectionEvent>>>>,
}

static GNS_GLOBAL: OnceLock<GnsGlobal> = OnceLock::new();

impl Drop for GnsGlobal {
    #[inline]
    fn drop(&mut self) {
        // Stop the service thread and tear down the internal state.
        //
        // GameNetworkingSockets does not support an init, kill, init cycle on
        // every version, so this runs only when the singleton itself is
        // dropped.
        unsafe { GameNetworkingSockets_Kill() }
    }
}

impl GnsGlobal {
    /// Returns a reference to the [`GnsGlobal`] instance.
    ///
    /// The first call initializes GameNetworkingSockets through
    /// [`sys::GameNetworkingSockets_Init`]. Later calls return the instance
    /// that call created.
    ///
    /// # Errors
    /// Returns [`GnsError::Init`] with the message that GameNetworkingSockets
    /// produced if initialization fails.
    pub fn get() -> GnsResult<&'static Self> {
        // Fast path: no lock
        if let Some(g) = GNS_GLOBAL.get() {
            return Ok(g);
        }
        // use get_or_try_init once stabilized: https://github.com/rust-lang/rust/issues/109737
        static INIT_LOCK: Mutex<()> = Mutex::new(());
        let _guard = INIT_LOCK.lock().unwrap();
        if let Some(g) = GNS_GLOBAL.get() {
            return Ok(g);
        }
        unsafe {
            let mut error: SteamDatagramErrMsg = MaybeUninit::zeroed().assume_init();
            if !GameNetworkingSockets_Init(core::ptr::null(), &mut error) {
                return Err(GnsError::Init(
                    CStr::from_ptr(error.as_ptr())
                        .to_str()
                        .unwrap_or("")
                        .to_owned(),
                ));
            }
        }
        let _ = GNS_GLOBAL.set(GnsGlobal {
            utils: GnsUtils(()),
            next_queue_id: AtomicI64::new(0),
            event_queues: RwLock::new(HashMap::new()),
        });
        Ok(GNS_GLOBAL.get().expect("impossible; qed;"))
    }

    #[inline]
    pub fn poll_callbacks(&self) {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_RunCallbacks(get_interface());
        }
    }

    #[inline]
    pub fn utils(&self) -> &GnsUtils {
        &self.utils
    }

    #[inline]
    pub fn queue_count(&self) -> usize {
        self.event_queues.read().unwrap().len()
    }

    #[inline]
    fn create_queue(&self) -> (i64, Arc<SegQueue<GnsConnectionEvent>>) {
        let queue = Arc::new(SegQueue::new());
        let queue_id = self.next_queue_id.fetch_add(1, Ordering::SeqCst);
        self.event_queues
            .write()
            .unwrap()
            .insert(queue_id, Arc::downgrade(&queue));
        (queue_id, queue)
    }
}

/// An opaque wrapper around [`sys::HSteamListenSocket`].
#[repr(transparent)]
pub(crate) struct GnsListenSocket(HSteamListenSocket);

/// An opaque wrapper around [`sys::HSteamNetPollGroup`].
#[repr(transparent)]
pub(crate) struct GnsPollGroup(HSteamNetPollGroup);

/// The initial state of a [`GnsSocket`].
///
/// A socket in this state is neither a client nor a server yet, so it holds no
/// data.
pub struct IsCreated;

mod private {
    pub trait Sealed {}
    impl Sealed for super::IsServer {}
    impl Sealed for super::IsClient {}
}

/// The operations that every ready [`GnsSocket`] supports.
///
/// A ready socket is either a client or a server. Both can read connection
/// events and receive messages.
pub trait IsReady: private::Sealed {
    /// Returns the connection event queue. The queue is thread-safe.
    fn queue(&self) -> &SegQueue<GnsConnectionEvent>;
    /// Receives up to `slots.len()` messages into `slots`.
    ///
    /// Returns the number of slots that GameNetworkingSockets filled, or
    /// [`GnsError::Receive`] if the underlying handle is invalid.
    fn receive(&self, slots: &mut [MaybeUninit<*mut ISteamNetworkingMessage>]) -> GnsResult<usize>;
}

/// The state of a [`GnsSocket`] that acts as a server, normally reached
/// through [`GnsSocket::listen`].
///
/// In this state the socket holds what it needs to accept connections and poll
/// them for messages.
pub struct IsServer {
    queue: Arc<SegQueue<GnsConnectionEvent>>,
    queue_id: i64,
    global: &'static GnsGlobal,
    listen_socket: GnsListenSocket,
    poll_group: GnsPollGroup,
}

impl Drop for IsServer {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_CloseListenSocket(
                get_interface(),
                self.listen_socket.0,
            );
            SteamAPI_ISteamNetworkingSockets_DestroyPollGroup(get_interface(), self.poll_group.0);
        }
        self.global
            .event_queues
            .write()
            .unwrap()
            .remove(&self.queue_id);
    }
}

impl IsReady for IsServer {
    #[inline]
    fn queue(&self) -> &SegQueue<GnsConnectionEvent> {
        &self.queue
    }

    fn receive(&self, slots: &mut [MaybeUninit<*mut ISteamNetworkingMessage>]) -> GnsResult<usize> {
        let result = unsafe {
            SteamAPI_ISteamNetworkingSockets_ReceiveMessagesOnPollGroup(
                get_interface(),
                self.poll_group.0,
                slots.as_mut_ptr() as _,
                slots.len() as _,
            ) as _
        };
        if result == usize::MAX {
            Err(GnsError::Receive)
        } else {
            Ok(result)
        }
    }
}

/// The state of a [`GnsSocket`] that acts as a client, normally reached
/// through [`GnsSocket::connect`].
///
/// In this state the socket holds what it needs to send and receive messages.
pub struct IsClient {
    queue: Arc<SegQueue<GnsConnectionEvent>>,
    queue_id: i64,
    global: &'static GnsGlobal,
    connection: GnsConnection,
}

impl Drop for IsClient {
    fn drop(&mut self) {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_CloseConnection(
                get_interface(),
                self.connection.0,
                0,
                core::ptr::null(),
                false,
            );
        }
        self.global
            .event_queues
            .write()
            .unwrap()
            .remove(&self.queue_id);
    }
}

impl IsReady for IsClient {
    #[inline]
    fn queue(&self) -> &SegQueue<GnsConnectionEvent> {
        &self.queue
    }

    fn receive(&self, slots: &mut [MaybeUninit<*mut ISteamNetworkingMessage>]) -> GnsResult<usize> {
        let result = unsafe {
            SteamAPI_ISteamNetworkingSockets_ReceiveMessagesOnConnection(
                get_interface(),
                self.connection.0,
                slots.as_mut_ptr() as _,
                slots.len() as _,
            ) as _
        };
        if result == usize::MAX {
            Err(GnsError::Receive)
        } else {
            Ok(result)
        }
    }
}

pub struct ToReceive(());

pub struct ToSend(());

/// A single receive slot.
///
/// Each slot is an uninitialized cell that GameNetworkingSockets fills with one
/// `*mut ISteamNetworkingMessage`. Build a buffer of slots, for example
/// `[const { MessageSlot::uninit() }; 128]`, and pass it to
/// [`GnsSocket::receive_messages_into`].
pub type MessageSlot = MaybeUninit<*mut ISteamNetworkingMessage>;

/// Rebuilds the owned message stored in `slot`.
///
/// # Safety
/// GameNetworkingSockets must have initialized `slot`, meaning the slot lies
/// within the prefix length that `receive` reported. The slot must not have
/// been taken already, otherwise the message is released more than once.
#[inline]
unsafe fn take_message(slot: &MessageSlot) -> GnsNetworkMessage<ToReceive> {
    GnsNetworkMessage(unsafe { slot.assume_init() }, PhantomData)
}

/// Tracks progress through a buffer of receive slots.
///
/// The slots in `slots[..len]` are initialized, and `pos` is the next slot to
/// hand out. This type holds the unsafe take and release logic in one place so
/// that the owning and borrowing iterators cannot drift apart.
struct SlotCursor {
    len: usize,
    pos: usize,
}

impl SlotCursor {
    fn next(&mut self, slots: &[MessageSlot]) -> Option<GnsNetworkMessage<ToReceive>> {
        if self.pos < self.len {
            // Safety: GameNetworkingSockets initialized `slots[..len]`, and
            // `pos` only increases, so each slot is taken at most once.
            let message = unsafe { take_message(&slots[self.pos]) };
            self.pos += 1;
            Some(message)
        } else {
            None
        }
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.len - self.pos
    }

    /// Releases every slot that was not handed out. Safe to call more than
    /// once.
    fn drain_unconsumed(&mut self, slots: &[MessageSlot]) {
        for slot in &slots[self.pos..self.len] {
            // Safety: same invariant as `next`. These slots are initialized
            // and were never handed out, so each is released exactly once.
            drop(unsafe { take_message(slot) });
        }
        self.pos = self.len;
    }
}

/// An iterator over the messages from one [`GnsSocket::receive_messages`]
/// call.
///
/// The iterator owns its `K`-slot pointer buffer inline, so it performs no heap
/// allocation. It yields each [`GnsNetworkMessage<ToReceive>`] by value, and
/// releases any message you did not consume when it is dropped.
///
/// See [`GnsSocket::receive_messages_into`] for a variant that borrows a buffer
/// you own, which also avoids moving the inline array.
pub struct ReceivedMessages<const K: usize> {
    slots: [MessageSlot; K],
    cursor: SlotCursor,
}

impl<const K: usize> Iterator for ReceivedMessages<K> {
    type Item = GnsNetworkMessage<ToReceive>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.next(&self.slots)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.cursor.remaining();
        (remaining, Some(remaining))
    }
}

impl<const K: usize> ExactSizeIterator for ReceivedMessages<K> {}

impl<const K: usize> core::iter::FusedIterator for ReceivedMessages<K> {}

impl<const K: usize> Drop for ReceivedMessages<K> {
    #[inline]
    fn drop(&mut self) {
        self.cursor.drain_unconsumed(&self.slots);
    }
}

/// An iterator returned by [`GnsSocket::receive_messages_into`].
///
/// The iterator borrows your buffer for its whole lifetime, so you cannot reuse
/// the buffer while messages are still outstanding. It yields each
/// [`GnsNetworkMessage<ToReceive>`] by value.
///
/// Nothing is allocated and the pointer buffer never moves. Only the individual
/// message pointers move. Any message you did not consume is released when the
/// iterator is dropped.
pub struct ReceivedMessagesInto<'a> {
    slots: &'a mut [MessageSlot],
    cursor: SlotCursor,
}

impl Iterator for ReceivedMessagesInto<'_> {
    type Item = GnsNetworkMessage<ToReceive>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.next(self.slots)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.cursor.remaining();
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ReceivedMessagesInto<'_> {}

impl core::iter::FusedIterator for ReceivedMessagesInto<'_> {}

impl Drop for ReceivedMessagesInto<'_> {
    #[inline]
    fn drop(&mut self) {
        self.cursor.drain_unconsumed(self.slots);
    }
}

bitflags::bitflags! {
    /// A type-safe wrapper over the `k_nSteamNetworkingSend_*` flags.
    ///
    /// The bit values match the raw `c_int` constants.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct SendFlags: i32 {
        const UNRELIABLE                  = sys::k_nSteamNetworkingSend_Unreliable;
        const NO_NAGLE                    = sys::k_nSteamNetworkingSend_NoNagle;
        const NO_DELAY                    = sys::k_nSteamNetworkingSend_NoDelay;
        const RELIABLE                    = sys::k_nSteamNetworkingSend_Reliable;
        const USE_CURRENT_THREAD          = sys::k_nSteamNetworkingSend_UseCurrentThread;
        const AUTO_RESTART_BROKEN_SESSION = sys::k_nSteamNetworkingSend_AutoRestartBrokenSession;
    }
}

/// A connection lane.
///
/// `priority` is a signed C `int` where a lower value means a higher priority.
/// `weight` is the relative scheduling weight within one priority class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GnsLane {
    pub priority: i32,
    pub weight: u16,
}

impl GnsLane {
    #[inline]
    pub const fn new(priority: i32, weight: u16) -> Self {
        Self { priority, weight }
    }
}

/// A lane identifier.
pub type GnsLaneId = u16;

/// The result of one message in a [`GnsSocket::send_messages`] batch.
///
/// `Skipped` mirrors how GameNetworkingSockets handles a failed batch. Once a
/// message fails on a connection, every later message in the same batch that
/// targets that connection is skipped without being attempted, and its result
/// is reported as `0`. A skipped message keeps its payload, so the wrapper
/// returns it to you, the same way it returns a `Failed` message.
#[must_use = "Failed/Skipped variants own a message that needs inspection or drop"]
pub enum SendOutcome {
    Sent(GnsMessageNumber),
    Failed(EResult, GnsNetworkMessage<ToSend>),
    Skipped(GnsNetworkMessage<ToSend>),
}

/// An owned byte buffer for an outbound message.
///
/// GameNetworkingSockets reads `m_pData` on its service thread after
/// `SendMessages` returns, so a message must own its bytes until
/// GameNetworkingSockets releases it.
///
/// [`into_raw`](Self::into_raw) returns a `(pointer, length)` pair that the
/// wrapper stores unchanged in `m_pData` and `m_cbSize`. When
/// GameNetworkingSockets releases the message, the wrapper passes those same
/// values to [`from_raw`](Self::from_raw) to rebuild `Self`, then drops the
/// result.
///
/// This mirrors `Box::into_raw` and `Box::from_raw`, so you can express how the
/// buffer is freed with an ordinary Rust `Drop` implementation.
///
/// # Safety
/// `from_raw(p, n)` must be sound whenever `(p, n)` came from an earlier
/// `into_raw` call on the same implementation. In other words, `from_raw` must
/// undo `into_raw` exactly.
///
/// `into_raw` must not run the `Drop` implementation of `Self`, because
/// ownership passes to GameNetworkingSockets.
pub unsafe trait Payload: Send + 'static {
    fn into_raw(self) -> (*mut u8, usize);
    /// # Safety
    /// `ptr` and `len` must be the values that an earlier
    /// [`into_raw`](Self::into_raw) call on this same implementation returned,
    /// and that ownership must not have been reclaimed already.
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self;
}

/// The `m_pfnFreeData` callback that the wrapper installs on every
/// `GnsNetworkMessage<ToSend>`.
///
/// It reads `m_pData` and `m_cbSize`, rebuilds `P` with
/// [`Payload::from_raw`], and drops the result.
extern "C" fn free_payload<P: Payload>(msg: *mut ISteamNetworkingMessage) {
    let ptr = unsafe { (*msg).m_pData } as *mut u8;
    let len = unsafe { (*msg).m_cbSize } as usize;
    // Safety: `GnsNetworkMessage::<ToSend>::new` wrote `ptr` and `len` from
    // `P::into_raw`, and GameNetworkingSockets releases each message once.
    drop(unsafe { P::from_raw(ptr, len) });
}

unsafe impl Payload for Box<[u8]> {
    #[inline]
    fn into_raw(self) -> (*mut u8, usize) {
        let len = self.len();
        let raw = Box::into_raw(self) as *mut u8;
        (raw, len)
    }
    #[inline]
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        let slice = core::ptr::slice_from_raw_parts_mut(ptr, len);
        unsafe { Box::from_raw(slice) }
    }
}

// This goes through `Box<[u8]>`. `into_boxed_slice` shrinks the buffer to fit,
// which costs one reallocation when the capacity differs from the length, so
// the pointer and length are enough to rebuild the value.
unsafe impl Payload for Vec<u8> {
    #[inline]
    fn into_raw(self) -> (*mut u8, usize) {
        <Box<[u8]> as Payload>::into_raw(self.into_boxed_slice())
    }
    #[inline]
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        unsafe { Vec::from_raw_parts(ptr, len, len) }
    }
}

unsafe impl Payload for String {
    #[inline]
    fn into_raw(self) -> (*mut u8, usize) {
        <Vec<u8> as Payload>::into_raw(self.into_bytes())
    }
    #[inline]
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        unsafe { String::from_raw_parts(ptr, len, len) }
    }
}

unsafe impl Payload for Arc<[u8]> {
    #[inline]
    fn into_raw(self) -> (*mut u8, usize) {
        let len = self.len();
        let raw = Arc::into_raw(self) as *const u8 as *mut u8;
        (raw, len)
    }
    #[inline]
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        let slice = core::ptr::slice_from_raw_parts(ptr as *const u8, len);
        unsafe { Arc::from_raw(slice) }
    }
}

unsafe impl Payload for &'static [u8] {
    #[inline]
    fn into_raw(self) -> (*mut u8, usize) {
        (self.as_ptr() as *mut u8, self.len())
    }
    #[inline]
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        unsafe { core::slice::from_raw_parts(ptr as *const u8, len) }
    }
}

unsafe impl Payload for &'static str {
    #[inline]
    fn into_raw(self) -> (*mut u8, usize) {
        (self.as_ptr() as *mut u8, self.len())
    }
    #[inline]
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }
}

/// A GameNetworkingSockets message, tagged with its direction.
///
/// The library produces `ToReceive` messages. You create `ToSend` messages with
/// [`GnsUtils::allocate_message`], and they own their payload through
/// [`Payload`]. Both kinds are released when dropped.
#[repr(transparent)]
pub struct GnsNetworkMessage<T>(*mut ISteamNetworkingMessage, PhantomData<T>);

impl<T> Drop for GnsNetworkMessage<T> {
    #[inline]
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                SteamAPI_SteamNetworkingMessage_t_Release(self.0);
            }
        }
    }
}

impl<T> GnsNetworkMessage<T> {
    /// Returns the raw `*mut ISteamNetworkingMessage` and forgets the wrapper.
    ///
    /// # Safety
    /// You take over releasing the message, for example by calling
    /// `SteamAPI_SteamNetworkingMessage_t_Release`. For a `ToSend` message that
    /// release also runs the `m_pfnFreeData` callback that [`Payload`]
    /// installed.
    #[inline]
    pub unsafe fn into_inner(self) -> *mut ISteamNetworkingMessage {
        // Hold off the wrapper's destructor. Releasing the message is now
        // the caller's job. Without this the message would be released here
        // and the returned pointer would dangle.
        core::mem::ManuallyDrop::new(self).0
    }

    #[inline]
    pub fn payload(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts((*self.0).m_pData as *const u8, (*self.0).m_cbSize as _)
        }
    }

    #[inline]
    pub fn message_number(&self) -> u64 {
        unsafe { (*self.0).m_nMessageNumber as _ }
    }

    /// Returns the local timestamp at which the message arrived, in
    /// microseconds.
    ///
    /// The value shares its timebase with [`GnsUtils::local_timestamp`], so
    /// compare the two to compute how long a message sat in the receive queue.
    /// It is meaningless on a message you allocated yourself.
    #[inline]
    pub fn time_received(&self) -> SteamNetworkingMicroseconds {
        unsafe { (*self.0).m_usecTimeReceived }
    }

    #[inline]
    pub fn lane(&self) -> GnsLaneId {
        unsafe { (*self.0).m_idxLane }
    }

    #[inline]
    pub fn flags(&self) -> SendFlags {
        SendFlags::from_bits_retain(unsafe { (*self.0).m_nFlags })
    }

    #[inline]
    pub fn user_data(&self) -> u64 {
        unsafe { (*self.0).m_nUserData as _ }
    }

    #[inline]
    pub fn connection(&self) -> GnsConnection {
        GnsConnection(unsafe { (*self.0).m_conn })
    }

    #[inline]
    pub fn connection_user_data(&self) -> u64 {
        unsafe { (*self.0).m_nConnUserData as _ }
    }
}

impl GnsNetworkMessage<ToSend> {
    #[inline]
    fn new<P: Payload>(
        ptr: *mut ISteamNetworkingMessage,
        conn: GnsConnection,
        flags: SendFlags,
        payload: P,
    ) -> Self {
        let (data_ptr, len) = payload.into_raw();
        unsafe {
            (*ptr).m_pData = data_ptr as *mut c_void;
            (*ptr).m_cbSize = len as i32;
            (*ptr).m_pfnFreeData = Some(free_payload::<P>);
        }
        GnsNetworkMessage(ptr, PhantomData)
            .set_flags(flags)
            .set_connection(conn)
    }

    #[inline]
    pub fn set_connection(self, GnsConnection(conn): GnsConnection) -> Self {
        unsafe { (*self.0).m_conn = conn }
        self
    }

    #[inline]
    pub fn set_lane(self, lane: GnsLaneId) -> Self {
        unsafe { (*self.0).m_idxLane = lane }
        self
    }

    #[inline]
    pub fn set_flags(self, flags: SendFlags) -> Self {
        unsafe { (*self.0).m_nFlags = flags.bits() as _ }
        self
    }

    #[inline]
    pub fn set_user_data(self, userdata: u64) -> Self {
        unsafe { (*self.0).m_nUserData = userdata as _ }
        self
    }
}

#[repr(transparent)]
#[derive(Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GnsConnection(HSteamNetConnection);

impl GnsConnection {
    /// Wraps a raw `HSteamNetConnection` handle.
    ///
    /// GameNetworkingSockets validates the handle when you use it. It rejects
    /// any handle that does not match a live connection.
    #[inline]
    pub const fn from_raw(handle: HSteamNetConnection) -> Self {
        Self(handle)
    }

    /// Returns `true` if this is not the invalid-connection value (`0`).
    #[inline]
    pub fn is_valid(self) -> bool {
        self.0 != k_HSteamNetConnection_Invalid
    }
}

#[derive(Default, Copy, Clone)]
pub struct GnsConnectionInfo(SteamNetConnectionInfo_t);

impl GnsConnectionInfo {
    #[inline]
    pub fn state(&self) -> ESteamNetworkingConnectionState {
        self.0.m_eState
    }

    #[inline]
    pub fn end_reason(&self) -> u32 {
        self.0.m_eEndReason as u32
    }

    #[inline]
    pub fn end_debug(&self) -> &str {
        unsafe { CStr::from_ptr(self.0.m_szEndDebug.as_ptr()) }
            .to_str()
            .unwrap_or("")
    }

    #[inline]
    pub fn remote_address(&self) -> IpAddr {
        ip_from_steam_ip_addr(&self.0.m_addrRemote)
    }

    #[inline]
    pub fn remote_port(&self) -> u16 {
        self.0.m_addrRemote.m_port
    }
}

#[derive(Debug, Default, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct GnsConnectionRealTimeLaneStatus(SteamNetConnectionRealTimeLaneStatus_t);

impl GnsConnectionRealTimeLaneStatus {
    #[inline]
    pub fn pending_bytes_unreliable(&self) -> u32 {
        self.0.m_cbPendingUnreliable as _
    }

    #[inline]
    pub fn pending_bytes_reliable(&self) -> u32 {
        self.0.m_cbPendingReliable as _
    }

    #[inline]
    pub fn bytes_sent_unacked_reliable(&self) -> u32 {
        self.0.m_cbSentUnackedReliable as _
    }

    #[inline]
    pub fn approximated_queue_time(&self) -> Duration {
        Duration::from_micros(self.0.m_usecQueueTime as _)
    }
}

#[derive(Default, Debug, Copy, Clone, PartialOrd, PartialEq)]
pub struct GnsConnectionRealTimeStatus(SteamNetConnectionRealTimeStatus_t);

impl GnsConnectionRealTimeStatus {
    #[inline]
    pub fn state(&self) -> ESteamNetworkingConnectionState {
        self.0.m_eState
    }

    #[inline]
    pub fn ping(&self) -> u32 {
        self.0.m_nPing as _
    }

    #[inline]
    pub fn quality_local(&self) -> f32 {
        self.0.m_flConnectionQualityLocal
    }

    #[inline]
    pub fn quality_remote(&self) -> f32 {
        self.0.m_flConnectionQualityRemote
    }

    #[inline]
    pub fn out_packets_per_sec(&self) -> f32 {
        self.0.m_flOutPacketsPerSec
    }

    #[inline]
    pub fn out_bytes_per_sec(&self) -> f32 {
        self.0.m_flOutBytesPerSec
    }

    #[inline]
    pub fn in_packets_per_sec(&self) -> f32 {
        self.0.m_flInPacketsPerSec
    }

    #[inline]
    pub fn in_bytes_per_sec(&self) -> f32 {
        self.0.m_flInBytesPerSec
    }

    #[inline]
    pub fn send_rate_bytes_per_sec(&self) -> u32 {
        self.0.m_nSendRateBytesPerSecond as _
    }

    #[inline]
    pub fn pending_bytes_unreliable(&self) -> u32 {
        self.0.m_cbPendingUnreliable as _
    }

    #[inline]
    pub fn pending_bytes_reliable(&self) -> u32 {
        self.0.m_cbPendingReliable as _
    }

    #[inline]
    pub fn bytes_sent_unacked_reliable(&self) -> u32 {
        self.0.m_cbSentUnackedReliable as _
    }

    #[inline]
    pub fn approximated_queue_time(&self) -> Duration {
        Duration::from_micros(self.0.m_usecQueueTime as _)
    }

    /// Returns the highest packet jitter seen since the last time you read this
    /// value. Reading it clears the high water mark.
    ///
    /// Returns `None` if no jitter data is available, which happens when the
    /// underlying value is negative or the connection type does not measure
    /// jitter.
    #[inline]
    pub fn max_jitter_usec(&self) -> Option<i32> {
        let val = self.0.m_usecMaxJitter;
        if val < 0 {
            None
        } else {
            Some(val)
        }
    }
}

#[derive(Default, Copy, Clone)]
pub struct GnsConnectionEvent(SteamNetConnectionStatusChangedCallback_t);

impl GnsConnectionEvent {
    #[inline]
    pub fn old_state(&self) -> ESteamNetworkingConnectionState {
        self.0.m_eOldState
    }

    #[inline]
    pub fn connection(&self) -> GnsConnection {
        GnsConnection(self.0.m_hConn)
    }

    #[inline]
    pub fn info(&self) -> GnsConnectionInfo {
        GnsConnectionInfo(self.0.m_info)
    }
}

/// A network socket, and the main type of this library.
///
/// Use [`GnsSocket::connect`] to create a client socket and
/// [`GnsSocket::listen`] to create a server socket. Every operation on a socket
/// is safe.
///
/// Dropping a socket frees everything that belongs to it. It does not free the
/// [`GnsGlobal`] instance.
pub struct GnsSocket<S> {
    global: &'static GnsGlobal,
    state: S,
}

impl<S> GnsSocket<S>
where
    S: IsReady,
{
    /// Returns the status of a connection and of its lanes.
    ///
    /// Configure the lanes with [`Self::configure_connection_lanes`] before you
    /// call this.
    pub fn get_connection_real_time_status(
        &self,
        GnsConnection(conn): GnsConnection,
        nb_of_lanes: u32,
    ) -> GnsResult<(
        GnsConnectionRealTimeStatus,
        Vec<GnsConnectionRealTimeLaneStatus>,
    )> {
        let mut lanes: Vec<GnsConnectionRealTimeLaneStatus> =
            vec![Default::default(); nb_of_lanes as _];
        let mut status: GnsConnectionRealTimeStatus = Default::default();
        check(unsafe {
            SteamAPI_ISteamNetworkingSockets_GetConnectionRealTimeStatus(
                get_interface(),
                conn,
                &mut status as *mut GnsConnectionRealTimeStatus
                    as *mut SteamNetConnectionRealTimeStatus_t,
                nb_of_lanes as _,
                lanes.as_mut_ptr() as *mut SteamNetConnectionRealTimeLaneStatus_t,
            )
        })?;
        Ok((status, lanes))
    }

    pub fn get_connection_info(
        &self,
        GnsConnection(conn): GnsConnection,
    ) -> Option<GnsConnectionInfo> {
        let mut info: SteamNetConnectionInfo_t = Default::default();
        if unsafe {
            SteamAPI_ISteamNetworkingSockets_GetConnectionInfo(get_interface(), conn, &mut info)
        } {
            Some(GnsConnectionInfo(info))
        } else {
            None
        }
    }

    /// Returns a verbose human-readable description of the state of a
    /// connection, intended for diagnostics and debug dumps.
    ///
    /// The format is subject to change between GameNetworkingSockets versions,
    /// so do not parse it. Returns `None` if the connection handle is invalid.
    pub fn get_detailed_connection_status(
        &self,
        GnsConnection(conn): GnsConnection,
    ) -> Option<String> {
        let mut buf = vec![0u8; 2048];
        loop {
            let result = unsafe {
                SteamAPI_ISteamNetworkingSockets_GetDetailedConnectionStatus(
                    get_interface(),
                    conn,
                    buf.as_mut_ptr() as *mut std::ffi::c_char,
                    buf.len() as _,
                )
            };
            if result < 0 {
                return None;
            }
            if result == 0 {
                let text = CStr::from_bytes_until_nul(&buf).ok()?;
                return Some(text.to_string_lossy().into_owned());
            }
            // The buffer was too small and `result` is the required size.
            buf.resize(result as usize, 0);
        }
    }

    /// Returns the debug name of a connection, previously set with
    /// [`Self::set_connection_name`].
    ///
    /// Returns `None` if the connection handle is invalid.
    pub fn get_connection_name(&self, GnsConnection(conn): GnsConnection) -> Option<String> {
        let mut buf = [0u8; 256];
        if unsafe {
            SteamAPI_ISteamNetworkingSockets_GetConnectionName(
                get_interface(),
                conn,
                buf.as_mut_ptr() as *mut std::ffi::c_char,
                buf.len() as _,
            )
        } {
            let name = CStr::from_bytes_until_nul(&buf).ok()?;
            Some(name.to_string_lossy().into_owned())
        } else {
            None
        }
    }

    /// Sets the debug name of a connection.
    ///
    /// The name shows up in diagnostics such as
    /// [`Self::get_detailed_connection_status`] and the debug output, which
    /// makes multi-connection logs much easier to read.
    ///
    /// # Errors
    /// Returns [`GnsError::Config`] if `name` contains an interior NUL byte.
    pub fn set_connection_name(
        &self,
        GnsConnection(conn): GnsConnection,
        name: &str,
    ) -> GnsResult<()> {
        let c = CString::new(name).map_err(|_| GnsError::Config("interior NUL"))?;
        unsafe {
            SteamAPI_ISteamNetworkingSockets_SetConnectionName(get_interface(), conn, c.as_ptr());
        }
        Ok(())
    }

    pub fn flush_messages_on_connection(
        &self,
        GnsConnection(conn): GnsConnection,
    ) -> GnsResult<()> {
        check(unsafe {
            SteamAPI_ISteamNetworkingSockets_FlushMessagesOnConnection(get_interface(), conn)
        })
    }

    /// Closes a connection.
    ///
    /// The wrapper forwards `debug` to the peer when you pass `Some`. Pass
    /// `None` to send no diagnostic string and avoid allocating.
    ///
    /// # Errors
    /// Returns [`GnsError::Close`] if the connection handle is invalid, for
    /// example because the connection is already closed.
    pub fn close_connection(
        &self,
        GnsConnection(conn): GnsConnection,
        reason: u32,
        debug: Option<&CStr>,
        linger: bool,
    ) -> GnsResult<()> {
        let debug_ptr = debug.map(|d| d.as_ptr()).unwrap_or(core::ptr::null());
        if unsafe {
            SteamAPI_ISteamNetworkingSockets_CloseConnection(
                get_interface(),
                conn,
                reason as _,
                debug_ptr,
                linger,
            )
        } {
            Ok(())
        } else {
            Err(GnsError::Close)
        }
    }

    /// Receives up to `K` messages and returns an iterator over the ones that
    /// were available.
    ///
    /// Each message is yielded by value, so you can keep it, forward it, or let
    /// it drop, which releases it. Any message left in the iterator is released
    /// when the iterator is dropped.
    ///
    /// The `K`-slot pointer buffer lives inline in the returned iterator, so
    /// this call allocates nothing and copies no payload. Use
    /// [`receive_messages_into`](Self::receive_messages_into) to reuse one
    /// buffer across calls and avoid moving the inline array.
    ///
    /// # Errors
    /// Returns [`GnsError::Receive`] if the connection or poll group handle is
    /// invalid.
    pub fn receive_messages<const K: usize>(&self) -> GnsResult<ReceivedMessages<K>> {
        let mut slots: [MessageSlot; K] = [const { MessageSlot::uninit() }; K];
        let len = self.state.receive(&mut slots)?;
        Ok(ReceivedMessages {
            slots,
            cursor: SlotCursor { len, pos: 0 },
        })
    }

    /// Receives up to `buffer.len()` messages into a buffer you own, and
    /// returns an iterator over the ones that were available.
    ///
    /// This is the variant of [`receive_messages`](Self::receive_messages) that
    /// neither allocates nor moves the buffer. GameNetworkingSockets fills
    /// `buffer` in place and the returned iterator borrows it, so reusing one
    /// buffer across a polling loop costs nothing per call.
    ///
    /// # Errors
    /// Returns [`GnsError::Receive`] if the connection or poll group handle is
    /// invalid.
    pub fn receive_messages_into<'a>(
        &self,
        buffer: &'a mut [MessageSlot],
    ) -> GnsResult<ReceivedMessagesInto<'a>> {
        let len = self.state.receive(buffer)?;
        Ok(ReceivedMessagesInto {
            slots: buffer,
            cursor: SlotCursor { len, pos: 0 },
        })
    }

    /// Returns an iterator that drains the pending connection events.
    ///
    /// Unlike [`receive_messages`](Self::receive_messages), you supply no
    /// buffer. Events arrive on an internal lock-free queue that the
    /// connection-status callback fills, and this call pops from that queue.
    pub fn receive_events(&self) -> impl Iterator<Item = GnsConnectionEvent> + '_ {
        core::iter::from_fn(|| self.state.queue().pop())
    }

    pub fn configure_connection_lanes(
        &self,
        GnsConnection(connection): GnsConnection,
        lanes: &[GnsLane],
    ) -> GnsResult<()> {
        let (priorities, weights): (Vec<i32>, Vec<u16>) =
            lanes.iter().map(|l| (l.priority, l.weight)).unzip();
        check(unsafe {
            SteamAPI_ISteamNetworkingSockets_ConfigureConnectionLanes(
                get_interface(),
                connection,
                lanes.len() as _,
                priorities.as_ptr(),
                weights.as_ptr(),
            )
        })
    }

    /// Sends a single message to its target connection.
    ///
    /// This is a convenience wrapper over
    /// [`send_messages`](Self::send_messages) for the common one-message case.
    pub fn send_message(&self, message: GnsNetworkMessage<ToSend>) -> GnsResult<GnsMessageNumber> {
        match self.send_messages(core::iter::once(message)).pop() {
            Some(SendOutcome::Sent(number)) => Ok(number),
            Some(SendOutcome::Failed(result, _)) => Err(GnsError::Api(result)),
            // A single message is never `Skipped`, because that only happens
            // to a message queued behind an earlier failure on the same
            // connection. `send_messages` also returns one outcome per input.
            _ => Err(GnsError::Api(EResult::k_EResultFail)),
        }
    }

    /// Sends each message to its target connection.
    ///
    /// The returned `Vec` holds one [`SendOutcome`] per input message, in the
    /// same order.
    pub fn send_messages(
        &self,
        messages: impl IntoIterator<Item = GnsNetworkMessage<ToSend>>,
    ) -> Vec<SendOutcome> {
        // Pass `bDeleteFailedMessages = false` so that the C library consumes
        // the messages it sends and leaves the failed and skipped ones for the
        // wrapper to wrap again. `ManuallyDrop` holds off the Rust destructor
        // across the FFI call.
        let mut raw: Vec<*mut ISteamNetworkingMessage> = messages
            .into_iter()
            .map(|message| {
                let message = core::mem::ManuallyDrop::new(message);
                message.0
            })
            .collect();
        let mut result = vec![0i64; raw.len()];
        unsafe {
            SteamAPI_ISteamNetworkingSockets_SendMessages(
                get_interface(),
                raw.len() as _,
                raw.as_mut_ptr(),
                result.as_mut_ptr(),
                false,
            );
        }
        result
            .into_iter()
            .zip(raw)
            .map(|(value, ptr)| {
                if value > 0 {
                    SendOutcome::Sent(value as _)
                } else if value < 0 {
                    // Sound because gns-sys pins GameNetworkingSockets as a
                    // submodule, so the generated `EResult` covers every value
                    // the library produces.
                    let result = unsafe { core::mem::transmute::<u32, EResult>((-value) as u32) };
                    SendOutcome::Failed(result, GnsNetworkMessage(ptr, PhantomData))
                } else {
                    SendOutcome::Skipped(GnsNetworkMessage(ptr, PhantomData))
                }
            })
            .collect()
    }
}

impl GnsSocket<IsCreated> {
    /// The C callback that GameNetworkingSockets invokes on a connection-state
    /// change.
    ///
    /// The wrapper stores the queue ID in the connection user data, which is
    /// how this callback finds the right queue in [`GnsGlobal`].
    unsafe extern "C" fn on_connection_state_changed(
        info: &mut SteamNetConnectionStatusChangedCallback_t,
    ) {
        let gns_global = GnsGlobal::get()
            // Reaching this point at all means GnsGlobal is initialized.
            .expect("GnsGlobal should be initialized");

        let queue_id = info.m_info.m_nUserData as _;
        // Fast path: take the read lock, look up the queue, and push if the
        // weak reference still upgrades.
        let needs_purge = {
            let queues = gns_global.event_queues.read().unwrap();
            match queues.get(&queue_id).and_then(Weak::upgrade) {
                Some(queue) => {
                    queue.push(GnsConnectionEvent(*info));
                    false
                }
                None => queues.contains_key(&queue_id),
            }
        };
        // Slow path: the socket was dropped while this callback ran, so the
        // entry is still in the map but the queue is gone. Take the write lock
        // to remove it. Queue IDs are never reused, so removing a key that
        // another thread already removed does no harm.
        if needs_purge {
            gns_global.event_queues.write().unwrap().remove(&queue_id);
        }
    }

    /// Creates a socket in the [`IsCreated`] state.
    #[inline]
    pub fn new(global: &'static GnsGlobal) -> Self {
        GnsSocket {
            global,
            state: IsCreated,
        }
    }

    fn setup_common(
        address: IpAddr,
        port: u16,
        queue_id: int64,
    ) -> (SteamNetworkingIPAddr, [SteamNetworkingConfigValue_t; 2]) {
        let addr = SteamNetworkingIPAddr {
            __bindgen_anon_1: match address {
                IpAddr::V4(address) => SteamNetworkingIPAddr__bindgen_ty_2 {
                    m_ipv4: SteamNetworkingIPAddr_IPv4MappedAddress {
                        m_8zeros: 0,
                        m_0000: 0,
                        m_ffff: 0xffff,
                        m_ip: address.octets(),
                    },
                },
                IpAddr::V6(address) => SteamNetworkingIPAddr__bindgen_ty_2 {
                    m_ipv6: address.octets(),
                },
            },
            m_port: port,
        };
        let options = [SteamNetworkingConfigValue_t {
            m_eDataType: ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Ptr,
            m_eValue: ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_Callback_ConnectionStatusChanged,
            m_val: SteamNetworkingConfigValue_t__bindgen_ty_1 {
              m_ptr: Self::on_connection_state_changed as *const fn(&SteamNetConnectionStatusChangedCallback_t) as *mut c_void
            }
          }, SteamNetworkingConfigValue_t {
            m_eDataType: ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Int64,
            m_eValue: ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_ConnectionUserData,
            m_val: SteamNetworkingConfigValue_t__bindgen_ty_1 {
              m_int64: queue_id
            }
        }];
        (addr, options)
    }

    /// Listens for incoming connections.
    ///
    /// This moves the socket from [`IsCreated`] to [`IsServer`], which gives
    /// you the server operations.
    pub fn listen(self, address: IpAddr, port: u16) -> GnsResult<GnsSocket<IsServer>> {
        let (queue_id, queue) = self.global.create_queue();
        let (addr, options) = Self::setup_common(address, port, queue_id);
        let listen_socket = unsafe {
            SteamAPI_ISteamNetworkingSockets_CreateListenSocketIP(
                get_interface(),
                &addr,
                options.len() as _,
                options.as_ptr(),
            )
        };
        if listen_socket == k_HSteamListenSocket_Invalid {
            Err(GnsError::Listen)
        } else {
            let poll_group =
                unsafe { SteamAPI_ISteamNetworkingSockets_CreatePollGroup(get_interface()) };
            if poll_group == k_HSteamNetPollGroup_Invalid {
                Err(GnsError::Listen)
            } else {
                Ok(GnsSocket {
                    global: self.global,
                    state: IsServer {
                        queue,
                        queue_id,
                        global: self.global,
                        listen_socket: GnsListenSocket(listen_socket),
                        poll_group: GnsPollGroup(poll_group),
                    },
                })
            }
        }
    }

    /// Connects to a remote host.
    ///
    /// This moves the socket from [`IsCreated`] to [`IsClient`], which gives
    /// you the client operations.
    pub fn connect(self, address: IpAddr, port: u16) -> GnsResult<GnsSocket<IsClient>> {
        let (queue_id, queue) = self.global.create_queue();
        let (addr, options) = Self::setup_common(address, port, queue_id);
        let connection = unsafe {
            SteamAPI_ISteamNetworkingSockets_ConnectByIPAddress(
                get_interface(),
                &addr,
                options.len() as _,
                options.as_ptr(),
            )
        };
        if connection == k_HSteamNetConnection_Invalid {
            Err(GnsError::Connect)
        } else {
            Ok(GnsSocket {
                global: self.global,
                state: IsClient {
                    queue,
                    queue_id,
                    global: self.global,
                    connection: GnsConnection(connection),
                },
            })
        }
    }

    /// Creates a pair of connections that talk to each other, mainly for
    /// tests and loopback communication between parts of one process.
    ///
    /// With `use_network_loopback` set, the traffic goes through the local
    /// network stack over `127.0.0.1`. Without it, the payloads take an
    /// internal shortcut. See `ISteamNetworkingSockets::CreateSocketPair` for
    /// the trade-offs.
    ///
    /// Both sockets come back in the [`IsClient`] state and already connected.
    /// The connections are created before the wrapper can attach its
    /// connection-state callback, so the initial transition to the connected
    /// state never shows up in [`GnsSocket::receive_events`]. Later events,
    /// such as the peer closing the connection, are delivered normally.
    pub fn socket_pair(
        self,
        use_network_loopback: bool,
    ) -> GnsResult<(GnsSocket<IsClient>, GnsSocket<IsClient>)> {
        let (queue_id_a, queue_a) = self.global.create_queue();
        let (queue_id_b, queue_b) = self.global.create_queue();
        let mut conn_a = k_HSteamNetConnection_Invalid;
        let mut conn_b = k_HSteamNetConnection_Invalid;
        let ok = unsafe {
            SteamAPI_ISteamNetworkingSockets_CreateSocketPair(
                get_interface(),
                &mut conn_a,
                &mut conn_b,
                use_network_loopback,
                core::ptr::null(),
                core::ptr::null(),
            )
        };
        if !ok {
            let mut queues = self.global.event_queues.write().unwrap();
            queues.remove(&queue_id_a);
            queues.remove(&queue_id_b);
            return Err(GnsError::SocketPair);
        }
        // Build the client states first so that their `Drop` implementations
        // close the connections and unregister the queues on any later error.
        let make_client = |connection, queue, queue_id| GnsSocket {
            global: self.global,
            state: IsClient {
                queue,
                queue_id,
                global: self.global,
                connection: GnsConnection(connection),
            },
        };
        let a = make_client(conn_a, queue_a, queue_id_a);
        let b = make_client(conn_b, queue_b, queue_id_b);
        // `CreateSocketPair` accepts no config options, so install the same
        // callback and queue ID that `setup_common` passes at creation time.
        for (conn, queue_id) in [(conn_a, queue_id_a), (conn_b, queue_id_b)] {
            unsafe {
                SteamAPI_ISteamNetworkingSockets_SetConnectionUserData(
                    get_interface(),
                    conn,
                    queue_id,
                );
            }
            self.global.utils().set_config_value_scoped(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_Callback_ConnectionStatusChanged,
                ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_Connection,
                conn as isize,
                GnsConfig::Ptr(Self::on_connection_state_changed as *const fn(&SteamNetConnectionStatusChangedCallback_t) as *mut c_void),
            )?;
        }
        Ok((a, b))
    }
}

impl GnsSocket<IsServer> {
    /// Accepts an incoming connection. Only a socket in the [`IsServer`] state
    /// has this operation.
    pub fn accept(&self, connection: GnsConnection) -> GnsResult<()> {
        check(unsafe {
            SteamAPI_ISteamNetworkingSockets_AcceptConnection(get_interface(), connection.0)
        })?;
        if !unsafe {
            SteamAPI_ISteamNetworkingSockets_SetConnectionPollGroup(
                get_interface(),
                connection.0,
                self.state.poll_group.0,
            )
        } {
            // The poll group and the connection should both be valid here, so
            // this is not expected to happen.
            return Err(GnsError::Accept);
        }
        Ok(())
    }

    /// Returns the address the listen socket is bound to.
    ///
    /// The address part is the unspecified address when the socket listens on
    /// every interface, which is what an all-zeros IP passed to
    /// [`GnsSocket::listen`] requests.
    pub fn get_listen_socket_address(&self) -> Option<(IpAddr, u16)> {
        let mut addr: SteamNetworkingIPAddr = unsafe { MaybeUninit::zeroed().assume_init() };
        if unsafe {
            SteamAPI_ISteamNetworkingSockets_GetListenSocketAddress(
                get_interface(),
                self.state.listen_socket.0,
                &mut addr,
            )
        } {
            Some((ip_from_steam_ip_addr(&addr), addr.m_port))
        } else {
            None
        }
    }

    /// Sets a configuration value on the listen socket, for example a
    /// connection option that every accepted connection inherits as its
    /// default.
    pub fn set_listen_socket_config_value(
        &self,
        typ: ESteamNetworkingConfigValue,
        value: GnsConfig<'_>,
    ) -> GnsResult<()> {
        self.global.utils().set_config_value_scoped(
            typ,
            ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_ListenSocket,
            self.state.listen_socket.0 as isize,
            value,
        )
    }

    /// Reads a configuration value back from the listen socket.
    pub fn get_listen_socket_config_value(
        &self,
        typ: ESteamNetworkingConfigValue,
    ) -> GnsResult<GnsConfigValue> {
        self.global.utils().get_config_value_scoped(
            typ,
            ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_ListenSocket,
            self.state.listen_socket.0 as isize,
        )
    }
}

impl GnsSocket<IsClient> {
    /// Returns the socket connection. Only a socket in the [`IsClient`] state
    /// has this operation.
    #[inline]
    pub fn connection(&self) -> GnsConnection {
        self.state.connection
    }
}

/// A configuration value for [`GnsUtils::set_global_config_value`] and
/// [`GnsUtils::set_connection_config_value`].
///
/// The enum is non-exhaustive so that variants for further
/// GameNetworkingSockets data types can be added. You can still construct
/// every existing variant.
#[non_exhaustive]
pub enum GnsConfig<'a> {
    Float(f32),
    Int32(i32),
    /// Allocates a `CString` so that the value ends in a NUL byte. Use
    /// [`GnsConfig::CStr`] to skip that allocation when you already hold a
    /// `CStr`.
    String(&'a str),
    /// A string variant that does not allocate, because `&CStr` already ends
    /// in a NUL byte.
    CStr(&'a CStr),
    Ptr(*mut c_void),
}

/// A configuration value read back through [`GnsUtils::get_global_config_value`]
/// and friends.
///
/// Unlike [`GnsConfig`], which borrows what you pass in, this type owns its
/// data, and it distinguishes `Int64` because GameNetworkingSockets stores
/// some values (such as connection user data) as 64-bit integers.
///
/// The enum is non-exhaustive because it mirrors the GameNetworkingSockets
/// data-type enum, which can grow.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum GnsConfigValue {
    Float(f32),
    Int32(i32),
    Int64(i64),
    String(String),
    Ptr(*mut c_void),
}

pub struct GnsUtils(());

type MsgPtr = *const ::std::os::raw::c_char;

/// A debug callback that you supply.
///
/// It must be `Send + Sync` because GameNetworkingSockets invokes it from its
/// service thread, and it may capture state that your own threads share.
type DebugCallback = dyn Fn(ESteamNetworkingSocketsDebugOutputType, &str) + Send + Sync + 'static;

/// Holds the callback that [`GnsUtils::enable_debug_output`] installs.
///
/// GameNetworkingSockets invokes it from its service thread, so this `OnceLock`
/// is the synchronization point.
static DEBUG_CB: OnceLock<Box<DebugCallback>> = OnceLock::new();

unsafe extern "C" fn debug_trampoline(ty: ESteamNetworkingSocketsDebugOutputType, msg: MsgPtr) {
    if let Some(cb) = DEBUG_CB.get() {
        let s = unsafe { CStr::from_ptr(msg) }.to_str().unwrap_or("");
        cb(ty, s);
    }
}

impl GnsUtils {
    /// Installs a debug callback.
    ///
    /// Only the first call takes effect. Later calls are ignored.
    ///
    /// GameNetworkingSockets runs the callback on its service thread, which is
    /// why the callback must be `Send + Sync + 'static`. The callback may
    /// capture state, because the wrapper stores it as a boxed closure. The
    /// `&str` is borrowed only for the duration of the call.
    pub fn enable_debug_output(
        &self,
        ty: ESteamNetworkingSocketsDebugOutputType,
        f: impl Fn(ESteamNetworkingSocketsDebugOutputType, &str) + Send + Sync + 'static,
    ) {
        let _ = DEBUG_CB.set(Box::new(f));
        unsafe {
            SteamAPI_ISteamNetworkingUtils_SetDebugOutputFunction(
                get_utils(),
                ty,
                Some(debug_trampoline),
            );
        }
    }

    /// Allocates an outbound message and takes ownership of `payload`.
    ///
    /// The buffer stays alive until GameNetworkingSockets releases the message.
    /// At that point the wrapper rebuilds `P` with [`Payload::from_raw`] and
    /// drops it. Nothing is copied when the payload already owns heap memory.
    #[inline]
    pub fn allocate_message<P: Payload>(
        &self,
        conn: GnsConnection,
        flags: SendFlags,
        payload: P,
    ) -> GnsNetworkMessage<ToSend> {
        let message_ptr = unsafe { SteamAPI_ISteamNetworkingUtils_AllocateMessage(get_utils(), 0) };
        GnsNetworkMessage::new(message_ptr, conn, flags, payload)
    }

    /// Sets a global configuration value, for example
    /// `k_ESteamNetworkingConfig_FakePacketLag_Send` to 1000 ms.
    pub fn set_global_config_value(
        &self,
        typ: ESteamNetworkingConfigValue,
        value: GnsConfig<'_>,
    ) -> GnsResult<()> {
        let result = match value {
            GnsConfig::Float(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueFloat(get_utils(), typ, x)
            },
            GnsConfig::Int32(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueInt32(get_utils(), typ, x)
            },
            GnsConfig::String(x) => {
                let c = CString::new(x).map_err(|_| GnsError::Config("interior NUL"))?;
                unsafe {
                    SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueString(
                        get_utils(),
                        typ,
                        c.as_ptr(),
                    )
                }
            }
            GnsConfig::CStr(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueString(
                    get_utils(),
                    typ,
                    x.as_ptr(),
                )
            },
            GnsConfig::Ptr(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValuePtr(get_utils(), typ, x)
            },
        };
        if result {
            Ok(())
        } else {
            Err(GnsError::Config("SetGlobalConfigValue rejected"))
        }
    }

    /// Sets a configuration value on one connection, for example
    /// `k_ESteamNetworkingConfig_SendRateMin` on an accepted connection.
    pub fn set_connection_config_value(
        &self,
        conn: GnsConnection,
        typ: ESteamNetworkingConfigValue,
        value: GnsConfig<'_>,
    ) -> GnsResult<()> {
        let result = match value {
            GnsConfig::Float(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetConnectionConfigValueFloat(
                    get_utils(),
                    conn.0,
                    typ,
                    x,
                )
            },
            GnsConfig::Int32(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetConnectionConfigValueInt32(
                    get_utils(),
                    conn.0,
                    typ,
                    x,
                )
            },
            GnsConfig::String(x) => {
                let c = CString::new(x).map_err(|_| GnsError::Config("interior NUL"))?;
                unsafe {
                    SteamAPI_ISteamNetworkingUtils_SetConnectionConfigValueString(
                        get_utils(),
                        conn.0,
                        typ,
                        c.as_ptr(),
                    )
                }
            }
            GnsConfig::CStr(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetConnectionConfigValueString(
                    get_utils(),
                    conn.0,
                    typ,
                    x.as_ptr(),
                )
            },
            GnsConfig::Ptr(x) => {
                return self.set_config_value_scoped(
                    typ,
                    ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_Connection,
                    conn.0 as isize,
                    GnsConfig::Ptr(x),
                )
            }
        };
        if result {
            Ok(())
        } else {
            Err(GnsError::Config("SetConnectionConfigValue rejected"))
        }
    }

    /// Sets a configuration value on any scope through the generic
    /// `SetConfigValue` entry point.
    fn set_config_value_scoped(
        &self,
        typ: ESteamNetworkingConfigValue,
        scope: ESteamNetworkingConfigScope,
        scope_obj: isize,
        value: GnsConfig<'_>,
    ) -> GnsResult<()> {
        // Owner that must outlive the FFI call.
        let owned_string;
        // `SetConfigValue` reads a string value directly from `pArg`, but a
        // pointer value through one level of indirection: `pArg` must point to
        // the pointer.
        let (data_type, arg): (ESteamNetworkingConfigDataType, *const c_void) = match &value {
            GnsConfig::Float(x) => (
                ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Float,
                x as *const f32 as _,
            ),
            GnsConfig::Int32(x) => (
                ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Int32,
                x as *const i32 as _,
            ),
            GnsConfig::String(x) => {
                owned_string = CString::new(*x).map_err(|_| GnsError::Config("interior NUL"))?;
                (
                    ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_String,
                    owned_string.as_ptr() as _,
                )
            }
            GnsConfig::CStr(x) => (
                ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_String,
                x.as_ptr() as _,
            ),
            GnsConfig::Ptr(x) => (
                ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Ptr,
                x as *const *mut c_void as _,
            ),
        };
        if unsafe {
            SteamAPI_ISteamNetworkingUtils_SetConfigValue(
                get_utils(),
                typ,
                scope,
                scope_obj,
                data_type,
                arg,
            )
        } {
            Ok(())
        } else {
            Err(GnsError::Config("SetConfigValue rejected"))
        }
    }

    /// Reads a configuration value from any scope.
    fn get_config_value_scoped(
        &self,
        typ: ESteamNetworkingConfigValue,
        scope: ESteamNetworkingConfigScope,
        scope_obj: isize,
    ) -> GnsResult<GnsConfigValue> {
        let mut data_type = ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Int32;
        let mut buf = vec![0u8; 64];
        let mut len = buf.len();
        loop {
            let result = unsafe {
                SteamAPI_ISteamNetworkingUtils_GetConfigValue(
                    get_utils(),
                    typ,
                    scope,
                    scope_obj,
                    &mut data_type,
                    buf.as_mut_ptr() as *mut c_void,
                    &mut len,
                )
            };
            match result {
                ESteamNetworkingGetConfigValueResult::k_ESteamNetworkingGetConfigValue_OK
                | ESteamNetworkingGetConfigValueResult::k_ESteamNetworkingGetConfigValue_OKInherited => {
                    break
                }
                ESteamNetworkingGetConfigValueResult::k_ESteamNetworkingGetConfigValue_BufferTooSmall => {
                    // `len` now holds the required size.
                    buf.resize(len, 0);
                }
                ESteamNetworkingGetConfigValueResult::k_ESteamNetworkingGetConfigValue_BadValue => {
                    return Err(GnsError::Config("unknown config value"))
                }
                ESteamNetworkingGetConfigValueResult::k_ESteamNetworkingGetConfigValue_BadScopeObj => {
                    return Err(GnsError::Config("bad scope object"))
                }
                _ => return Err(GnsError::Config("GetConfigValue failed")),
            }
        }
        // `buf` is a byte buffer, so read the typed values unaligned.
        let value = match data_type {
            ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Int32 => {
                GnsConfigValue::Int32(unsafe { (buf.as_ptr() as *const i32).read_unaligned() })
            }
            ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Int64 => {
                GnsConfigValue::Int64(unsafe { (buf.as_ptr() as *const i64).read_unaligned() })
            }
            ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Float => {
                GnsConfigValue::Float(unsafe { (buf.as_ptr() as *const f32).read_unaligned() })
            }
            ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_String => {
                let s = CStr::from_bytes_until_nul(&buf)
                    .map_err(|_| GnsError::Config("string value missing NUL"))?;
                GnsConfigValue::String(s.to_string_lossy().into_owned())
            }
            ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Ptr => {
                GnsConfigValue::Ptr(unsafe {
                    (buf.as_ptr() as *const *mut c_void).read_unaligned()
                })
            }
            _ => return Err(GnsError::Config("unknown config data type")),
        };
        Ok(value)
    }

    /// Reads a global configuration value back, the counterpart of
    /// [`Self::set_global_config_value`].
    #[inline]
    pub fn get_global_config_value(
        &self,
        typ: ESteamNetworkingConfigValue,
    ) -> GnsResult<GnsConfigValue> {
        self.get_config_value_scoped(
            typ,
            ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_Global,
            0,
        )
    }

    /// Reads a configuration value from one connection, the counterpart of
    /// [`Self::set_connection_config_value`].
    ///
    /// A value that was never set on the connection itself comes back from the
    /// enclosing scope, such as the listen socket or the global defaults.
    #[inline]
    pub fn get_connection_config_value(
        &self,
        conn: GnsConnection,
        typ: ESteamNetworkingConfigValue,
    ) -> GnsResult<GnsConfigValue> {
        self.get_config_value_scoped(
            typ,
            ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_Connection,
            conn.0 as isize,
        )
    }

    /// Returns the name, data type, and maximally specific scope of a
    /// configuration value, or `None` if the value is unknown.
    pub fn get_config_value_info(
        &self,
        typ: ESteamNetworkingConfigValue,
    ) -> Option<(
        &'static str,
        ESteamNetworkingConfigDataType,
        ESteamNetworkingConfigScope,
    )> {
        let mut data_type = ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Int32;
        let mut scope = ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_Global;
        let name = unsafe {
            SteamAPI_ISteamNetworkingUtils_GetConfigValueInfo(
                get_utils(),
                typ,
                &mut data_type,
                &mut scope,
            )
        };
        if name.is_null() {
            None
        } else {
            // The name is a static string inside GameNetworkingSockets.
            let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("");
            Some((name, data_type, scope))
        }
    }

    /// Returns the current local timestamp, in microseconds.
    ///
    /// The timebase starts at a value that will never be confused with an
    /// interval, and it matches [`GnsNetworkMessage::time_received`].
    #[inline]
    pub fn local_timestamp(&self) -> SteamNetworkingMicroseconds {
        unsafe { SteamAPI_ISteamNetworkingUtils_GetLocalTimestamp(get_utils()) }
    }
}
