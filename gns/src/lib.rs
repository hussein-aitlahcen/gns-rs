//! # Rust wrapper for Valve GameNetworkingSockets.
//!
//! Provides an abstraction over the low-level library.
//! There are multiple advantage to use this abstraction:
//! - Type safety: most of the low-level structures are wrapped and we leverage the type system to restrict the operations such that they are all **safe**.
//! - High level: the library abstract most of the structure in such a way that you don't have to deal with the low-level FFI plumbering required. The API is idiomatic, pure Rust.
//!
//! # Example
//!
//! ```
//! use gns::{GnsGlobal, GnsSocket, IsCreated};
//! use std::net::Ipv6Addr;
//! use std::time::Duration;
//!
//! // **uwrap** must be banned in production, we use it here to extract the most relevant part of the library.
//!
//! // Initial the global networking state. Note that this instance must be unique per-process.
//! let gns_global = GnsGlobal::get().unwrap();
//!
//! // Create a new [`GnsSocket`], the index type [`IsCreated`] is used to determine the state of the socket.
//! // The [`GnsSocket::new`] function is only available for the [`IsCreated`] state. This is the initial state of the socket.
//! let gns_socket = GnsSocket::<IsCreated>::new(gns_global);
//!
//! // Choose your own port
//! let port = 9001;
//!
//! // We now do a transition from [`IsCreated`] to the [`IsClient`] state. The [`GnsSocket::connect`] operation does this transition for us.
//! // Since we are now using a client socket, we have access to a different set of operations.
//! let client = gns_socket.connect(Ipv6Addr::LOCALHOST.into(), port).unwrap();
//!
//! // Now that we initiated a connection, there is three operation we must loop over:
//! // - polling for new messages
//! // - polling for connection status change
//! // - polling for callbacks (low-level callbacks required by the underlying library).
//! // Important to know, regardless of the type of socket, whether it is in [`IsClient`] or [`IsServer`] state, theses three operations are the same.
//! // The only difference is that polling for messages and status on the client only act on the client connection, while polling for messages and status on a server yield event for all connected clients.
//!
//! // You would loop on the below code.
//! // Run the low-level callbacks.
//! gns_global.poll_callbacks();
//!
//! // Receive a maximum of 100 messages on the client connection.
//! // For each messages, print it's payload.
//! let _actual_nb_of_messages_processed = client.poll_messages::<100>(|message| {
//!   println!("{}", core::str::from_utf8(message.payload()).unwrap());
//! });
//!
//! // Don't do anything with events.
//! // One would check the event for connection status, i.e. doing something when we are connected/disconnected from the server.
//! let _actual_nb_of_events_processed = client.poll_event::<100>(|_| {
//! });
//!
//! // Sleep a little bit.
//! std::thread::sleep(Duration::from_millis(10))
//! ```
//!
//! # Note
//!
//! Each [`GnsSocket`] registers a [`Weak<SegQueue<GnsConnectionEvent>>`] in [`GnsGlobal`]'s queue map so that incoming connection-state callbacks can find their owner. The entry is removed when the socket is dropped.

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

fn get_interface() -> *mut ISteamNetworkingSockets {
    unsafe { SteamAPI_SteamNetworkingSockets_v009() }
}

fn get_utils() -> *mut ISteamNetworkingUtils {
    unsafe { SteamAPI_SteamNetworkingUtils_v003() }
}

/// A network message number. Simple alias for documentation.
pub type GnsMessageNumber = u64;

/// Errors surfaced by the wrapper. Wraps Steam's [`EResult`] for API failures
/// and adds variants for setup paths that don't return an `EResult`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GnsError {
    #[error("GameNetworkingSockets_Init failed: {0}")]
    Init(String),
    #[error("listen failed: invalid handle")]
    Listen,
    #[error("connect failed: invalid handle")]
    Connect,
    #[error("steam api: {0:?}")]
    Api(EResult),
    #[error("config: {0}")]
    Config(&'static str),
}

pub type GnsResult<T> = Result<T, GnsError>;

/// Map an `EResult` returned by an FFI call to a [`GnsResult`].
#[inline]
fn check(e: EResult) -> GnsResult<()> {
    match e {
        EResult::k_EResultOK => Ok(()),
        e => Err(GnsError::Api(e)),
    }
}

/// Wraps the initialization/destruction of the low-level *GameNetworkingSockets* and associated
/// singletons.
///
/// A reference can be retrieved via [`GnsGlobal::get()`], which will initialize
/// *GameNetworkingSockets* if it has not yet been initialized.
pub struct GnsGlobal {
    utils: GnsUtils,
    next_queue_id: AtomicI64,
    /// Per-socket event-queue registry. Reads dominate (one lookup per
    /// connection-state callback from the GNS service thread); writes
    /// happen only on socket creation / drop and on the rare race where
    /// a callback fires for a just-dropped socket. `RwLock` lets future
    /// observability paths read concurrently without contending.
    event_queues: RwLock<HashMap<i64, Weak<SegQueue<GnsConnectionEvent>>>>,
}

static GNS_GLOBAL: OnceLock<GnsGlobal> = OnceLock::new();

impl Drop for GnsGlobal {
    fn drop(&mut self) {
        // Stop the GNS service thread and tear down internal state.
        // GNS does not support `_Init`/`_Kill`/`_Init` cycles across all
        // versions, so we only run this when the singleton itself is being
        // dropped (i.e. process exit / explicit static-clear in tests).
        unsafe { GameNetworkingSockets_Kill() }
    }
}

impl GnsGlobal {
    /// Try to acquire a reference to the [`GnsGlobal`] instance.
    ///
    /// If GnsGlobal has not yet been successfully initialized, a call to
    /// [`sys::GameNetworkingSockets_Init`] will be made. If successful, a reference to GnsGlobal
    /// will be returned.
    ///
    /// If GnsGlobal has already been initialized, this method returns a reference to the already
    /// created GnsGlobal instance.
    ///
    /// # Errors
    /// Returns [`GnsError::Init`] with the message produced by GNS if
    /// initialization fails.
    pub fn get() -> GnsResult<&'static Self> {
        // Fast path: no lock
        if let Some(g) = GNS_GLOBAL.get() {
            return Ok(g);
        }
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

/// Opaque wrapper around the low-level [`sys::HSteamListenSocket`].
#[repr(transparent)]
pub(crate) struct GnsListenSocket(HSteamListenSocket);

/// Opaque wrapper around the low-level [`sys::HSteamNetPollGroup`].
#[repr(transparent)]
pub(crate) struct GnsPollGroup(HSteamNetPollGroup);

/// Initial state of a [`GnsSocket`].
/// This state represent a socket that has not been used as a Server or Client implementation.
/// Consequently, the state is empty.
pub struct IsCreated;

mod private {
    pub trait Sealed {}
    impl Sealed for super::IsServer {}
    impl Sealed for super::IsClient {}
}

/// Common functions available for any [`GnsSocket`] state that is implementing it.
/// Regardless of being a client or server, a ready socket will allow us to query for connection events as well as receive messages.
pub trait IsReady: private::Sealed {
    /// Return a reference to the connection event queue. The queue is thread-safe.
    fn queue(&self) -> &SegQueue<GnsConnectionEvent>;
    /// Receive up to `K` messages into `slots`. Returns the count actually
    /// initialized by GNS, or `usize::MAX` if the C call signaled an error.
    fn receive<const K: usize>(
        &self,
        slots: &mut [MaybeUninit<*mut ISteamNetworkingMessage>; K],
    ) -> usize;
}

/// State of a [`GnsSocket`] that has been determined to be a server, usually via the [`GnsSocket::listen`] call.
/// In this state, the socket hold the data required to accept connections and poll them for messages.
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

    #[inline]
    fn receive<const K: usize>(
        &self,
        slots: &mut [MaybeUninit<*mut ISteamNetworkingMessage>; K],
    ) -> usize {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_ReceiveMessagesOnPollGroup(
                get_interface(),
                self.poll_group.0,
                slots.as_mut_ptr() as _,
                K as _,
            ) as _
        }
    }
}

/// State of a [`GnsSocket`] that has been determined to be a client, usually via the [`GnsSocket::connect`] call.
/// In this state, the socket hold the data required to receive and send messages.
pub struct IsClient {
    queue: Arc<SegQueue<GnsConnectionEvent>>,
    queue_id: i64,
    global: &'static GnsGlobal,
    connection: GnsConnection,
}

impl Drop for IsClient {
    #[inline]
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

    #[inline]
    fn receive<const K: usize>(
        &self,
        slots: &mut [MaybeUninit<*mut ISteamNetworkingMessage>; K],
    ) -> usize {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_ReceiveMessagesOnConnection(
                get_interface(),
                self.connection.0,
                slots.as_mut_ptr() as _,
                K as _,
            ) as _
        }
    }
}

pub struct ToReceive(());

pub struct ToSend(());

bitflags::bitflags! {
    /// Type-safe wrapper over the GNS `k_nSteamNetworkingSend_*` flags.
    /// Carries the same bit values as the raw `c_int` constants.
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

/// A connection lane: priority (lower = higher priority, signed `int` in C)
/// and weight (relative scheduling weight within a priority class).
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

/// A lane Id.
pub type GnsLaneId = u16;

/// Outcome of an individual message inside a [`GnsSocket::send_messages`] batch.
///
/// `Skipped` reflects GNS's batched-failure semantic: when a message earlier
/// in the same batch fails on connection X, every later message targeting X
/// is short-circuited (`pOutMessageNumberOrResult[i] = 0` per
/// `csteamnetworkingsockets.cpp:1364`). `m_pData` is *not* consumed in that
/// case, so we hand the original message back to the caller alongside
/// `Failed`.
#[must_use = "Failed/Skipped variants own a message that needs inspection or drop"]
pub enum SendOutcome {
    Sent(GnsMessageNumber),
    Failed(EResult, GnsNetworkMessage<ToSend>),
    Skipped(GnsNetworkMessage<ToSend>),
}

/// Owned byte buffer for outbound messages. GNS reads `m_pData`
/// asynchronously on its service thread after `SendMessages` returns,
/// so the message must own the bytes until GNS releases it.
///
/// `into_raw` returns `(ptr, len)` stored verbatim in `m_pData` /
/// `m_cbSize`. When GNS releases the message, the wrapper calls
/// [`from_raw`](Self::from_raw) with those same values to reconstruct
/// `Self`; the reconstructed value is then dropped.
///
/// This mirrors `Box::into_raw` / `Box::from_raw` and lets the
/// implementor express its free semantic via ordinary Rust `Drop`.
///
/// # Safety
/// `from_raw(p, n)` must be sound when `(p, n)` came from a previous
/// `into_raw` call on the same impl (i.e. `from_raw(into_raw(..))` must be an isomorphism).
/// The implementor must arrange for `into_raw` *not* to run `Self`'s `Drop`(because ownership is being transferred to GNS).
pub unsafe trait Payload: Send + 'static {
    fn into_raw(self) -> (*mut u8, usize);
    /// # Safety
    /// `ptr` and `len` must be the values returned by a prior
    /// [`into_raw`](Self::into_raw) call on this same impl, and that
    /// transferred ownership must not have already been reclaimed.
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self;
}

/// Monomorphized `m_pfnFreeData` callback installed for every
/// `GnsNetworkMessage<ToSend>`. Reads `m_pData` / `m_cbSize`,
/// reconstructs `P` via [`Payload::from_raw`], and lets `Drop` run.
extern "C" fn free_payload<P: Payload>(msg: *mut ISteamNetworkingMessage) {
    let ptr = unsafe { (*msg).m_pData } as *mut u8;
    let len = unsafe { (*msg).m_cbSize } as usize;
    // Safety: (ptr, len) were just written by `GnsNetworkMessage::<ToSend>::new`
    // from `P::into_raw`, and GNS releases each message at most once.
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

// Routes through `Box<[u8]>`: `into_boxed_slice` shrinks-to-fit (one
// realloc when `cap != len`) so `(ptr, len)` is enough to reconstruct.
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

/// Type-state-tagged GNS message. `ToReceive` instances are produced by
/// the library; `ToSend` instances are created via
/// [`GnsUtils::allocate_message`] and own their payload through
/// [`Payload`]. Both are released on drop.
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
    /// Extract the raw `*mut ISteamNetworkingMessage` and forget the wrapper.
    ///
    /// # Safety
    /// The caller takes over the message's release path: dropping the
    /// pointer's referent (e.g. via `SteamAPI_SteamNetworkingMessage_t_Release`)
    /// is now their responsibility. For `ToSend` messages this also means
    /// the `Payload`-installed `m_pfnFreeData` will run when the C side
    /// releases the message.
    #[inline]
    pub unsafe fn into_inner(self) -> *mut ISteamNetworkingMessage {
        self.0
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
    pub fn set_lane(self, lane: u16) -> Self {
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
    /// Wrap a raw `HSteamNetConnection` handle. Validity is enforced by GNS
    /// on use; an arbitrary handle that does not match a live connection
    /// will simply be rejected by the relevant API call.
    #[inline]
    pub const fn from_raw(handle: HSteamNetConnection) -> Self {
        Self(handle)
    }

    /// `true` if this is not the GNS invalid-connection sentinel (`0`).
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
        let ipv4 = unsafe { self.0.m_addrRemote.__bindgen_anon_1.m_ipv4 };
        if ipv4.m_8zeros == 0 && ipv4.m_0000 == 0 && ipv4.m_ffff == 0xffff {
            IpAddr::from(Ipv4Addr::from(ipv4.m_ip))
        } else {
            IpAddr::from(Ipv6Addr::from(unsafe {
                self.0.m_addrRemote.__bindgen_anon_1.m_ipv6
            }))
        }
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

    /// Returns the highest packet jitter experienced since the last time this
    /// information was fetched. The high water mark is cleared each time you
    /// fetch the info.
    ///
    /// Returns `None` if no jitter data is available (the underlying value is negative),
    /// or if the connection type doesn't support jitter measurement.
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

/// [`GnsSocket`] is the most important structure of this library.
/// This structure is used to create client ([`GnsSocket<IsClient>`]) and server ([`GnsSocket<IsServer>`]) sockets via the [`GnsSocket::connect`] and [`GnsSocket::listen`] functions.
/// The drop implementation make sure that everything related to this structure is correctly freed, except the [`GnsGlobal`] instance and the user has a strong guarantee that all the available operations over the socket are **safe**.
pub struct GnsSocket<S> {
    global: &'static GnsGlobal,
    state: S,
}

impl<S> GnsSocket<S>
where
    S: IsReady,
{
    /// Get a connection lane status.
    /// This call is possible only if lanes has been previously configured using configure_connection_lanes
    #[inline]
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

    #[inline]
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

    #[inline]
    pub fn flush_messages_on_connection(
        &self,
        GnsConnection(conn): GnsConnection,
    ) -> GnsResult<()> {
        check(unsafe {
            SteamAPI_ISteamNetworkingSockets_FlushMessagesOnConnection(get_interface(), conn)
        })
    }

    /// Close a connection. `pszDebug` is forwarded to the peer if non-`None`;
    /// pass `None` to send no diagnostic string and avoid all allocation.
    #[inline]
    pub fn close_connection(
        &self,
        GnsConnection(conn): GnsConnection,
        reason: u32,
        debug: Option<&CStr>,
        linger: bool,
    ) -> bool {
        let debug_ptr = debug.map(|d| d.as_ptr()).unwrap_or(core::ptr::null());
        unsafe {
            SteamAPI_ISteamNetworkingSockets_CloseConnection(
                get_interface(),
                conn,
                reason as _,
                debug_ptr,
                linger,
            )
        }
    }

    #[inline]
    pub fn poll_messages<const K: usize>(
        &self,
        mut message_callback: impl FnMut(&GnsNetworkMessage<ToReceive>),
    ) -> Option<usize> {
        // GNS writes raw `*mut SteamNetworkingMessage_t` into each slot.
        // We only `assume_init` the first `n` it actually wrote.
        let mut slots: [MaybeUninit<*mut ISteamNetworkingMessage>; K] =
            [const { MaybeUninit::uninit() }; K];
        let n = self.state.receive(&mut slots);
        if n == usize::MAX {
            return None;
        }
        for slot in &slots[..n] {
            // Safety: GNS guarantees the first `n` slots are initialized.
            let message =
                GnsNetworkMessage::<ToReceive>(unsafe { slot.assume_init() }, PhantomData);
            message_callback(&message);
            // `message` drops here, releasing the underlying GNS message.
        }
        Some(n)
    }

    #[inline]
    pub fn poll_event<const K: usize>(
        &self,
        mut event_callback: impl FnMut(GnsConnectionEvent),
    ) -> usize {
        let mut processed = 0;
        'a: while let Some(event) = self.state.queue().pop() {
            event_callback(event);
            processed += 1;
            if processed == K {
                break 'a;
            }
        }
        processed
    }

    #[inline]
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

    /// Dispatch each message to its target connection. See [`SendOutcome`]
    /// for the per-message result shape.
    #[inline]
    pub fn send_messages(&self, messages: Vec<GnsNetworkMessage<ToSend>>) -> Vec<SendOutcome> {
        // `bDeleteFailedMessages = false`: C consumes successful messages
        // and leaves the failed (or skipped) ones for us to re-wrap.
        // `ManuallyDrop` suspends our destructor across the FFI call.
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
                    // Sound: gns-sys is a pinned static submodule so the
                    // bindgen `EResult` mirrors every value GNS produces.
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
    /// Unsafe, C-like callback, we use the user data to pass the queue ID, so we can find the
    /// correct queue in GnsGlobal.
    unsafe extern "C" fn on_connection_state_changed(
        info: &mut SteamNetConnectionStatusChangedCallback_t,
    ) {
        let gns_global = GnsGlobal::get()
            // GnsGlobal needs to be initialized to even reach this point in the first place.
            .expect("GnsGlobal should be initialized");

        let queue_id = info.m_info.m_nUserData as _;
        // Hot path: take the read lock, look up, push if upgradeable.
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
        // Cold path: race with socket drop — the entry is still in the
        // map but the queue is gone. Escalate to a write lock to purge.
        // `queue_id`s are monotonic (no reuse), so removing a no-longer-
        // present key is harmless if another thread beat us to it.
        if needs_purge {
            gns_global.event_queues.write().unwrap().remove(&queue_id);
        }
    }

    /// Initialize a new socket in [`IsCreated`] state.
    #[inline]
    pub fn new(global: &'static GnsGlobal) -> Self {
        GnsSocket {
            global,
            state: IsCreated,
        }
    }

    #[inline]
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

    /// Listen for incoming connections, the socket transition from [`IsCreated`] to [`IsServer`], allowing a new set of server operations.
    #[inline]
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

    /// Connect to a remote host, the socket transition from [`IsCreated`] to [`IsClient`], allowing a new set of client operations.
    #[inline]
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
}

impl GnsSocket<IsServer> {
    /// Accept an incoming connection. This operation is available only if the socket is in the [`IsServer`] state.
    #[inline]
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
            panic!("It's impossible not to be able to set the connection poll group as both the poll group and the connection must be valid at this point.");
        }
        Ok(())
    }
}

impl GnsSocket<IsClient> {
    /// Return the socket connection. This operation is available only if the socket is in the [`IsClient`] state.
    #[inline]
    pub fn connection(&self) -> GnsConnection {
        self.state.connection
    }
}

/// The configuration value used to define configure global variables in [`GnsUtils::set_global_config_value`]
pub enum GnsConfig<'a> {
    Float(f32),
    Int32(u32),
    /// Allocates a `CString` to enforce NUL-termination. Use [`GnsConfig::CStr`]
    /// to skip the allocation when you already have a `CStr`.
    String(&'a str),
    /// Zero-allocation string variant; `&CStr` already carries a trailing NUL.
    CStr(&'a CStr),
    Ptr(*mut c_void),
}

pub struct GnsUtils(());

type MsgPtr = *const ::std::os::raw::c_char;

/// User-supplied debug callback. Set once via [`GnsUtils::enable_debug_output`];
/// invoked from the GNS service thread, so the underlying `OnceLock` is the
/// synchronization point.
static DEBUG_CB: OnceLock<fn(ESteamNetworkingSocketsDebugOutputType, &str)> = OnceLock::new();

unsafe extern "C" fn debug_trampoline(ty: ESteamNetworkingSocketsDebugOutputType, msg: MsgPtr) {
    if let Some(cb) = DEBUG_CB.get() {
        let s = unsafe { CStr::from_ptr(msg) }.to_str().unwrap_or("");
        cb(ty, s);
    }
}

impl GnsUtils {
    /// Install a debug callback. Subsequent calls are silently ignored —
    /// only the first registration wins. The callback runs on GNS's service
    /// thread; the `&str` is borrowed for the call duration only.
    #[inline]
    pub fn enable_debug_output(
        &self,
        ty: ESteamNetworkingSocketsDebugOutputType,
        f: fn(ESteamNetworkingSocketsDebugOutputType, &str),
    ) {
        let _ = DEBUG_CB.set(f);
        unsafe {
            SteamAPI_ISteamNetworkingUtils_SetDebugOutputFunction(
                get_utils(),
                ty,
                Some(debug_trampoline),
            );
        }
    }

    /// Allocate a new outbound message, taking ownership of `payload`.
    /// The buffer is held until GNS releases the message, at which point
    /// the wrapper reconstructs `P` via [`Payload::from_raw`] and lets
    /// its `Drop` run. Zero-copy for already-owned heap buffers.
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

    /// Set a global configuration value, i.e. k_ESteamNetworkingConfig_FakePacketLag_Send => 1000 ms
    #[inline]
    pub fn set_global_config_value<'a>(
        &self,
        typ: ESteamNetworkingConfigValue,
        value: GnsConfig<'a>,
    ) -> GnsResult<()> {
        let result = match value {
            GnsConfig::Float(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueFloat(get_utils(), typ, x)
            },
            GnsConfig::Int32(x) => unsafe {
                SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueInt32(get_utils(), typ, x as i32)
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

    /// Set a per-connection configuration value, e.g. k_ESteamNetworkingConfig_SendRateMin/Max on an individual accepted connection
    #[inline]
    pub fn set_connection_config_value<'a>(
        &self,
        conn: GnsConnection,
        typ: ESteamNetworkingConfigValue,
        value: GnsConfig<'a>,
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
                    x as i32,
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
            GnsConfig::Ptr(_) => return Err(GnsError::Config("Ptr not supported per-connection")),
        };
        if result {
            Ok(())
        } else {
            Err(GnsError::Config("SetConnectionConfigValue rejected"))
        }
    }
}
