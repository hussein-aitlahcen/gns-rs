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
//! // **uwrap** must be banned in production, we use it here to extract the most relevant part of the library.
//!
//! // Initial the global networking state. Note that this instance must be unique per-process.
//! let gns_global = GnsGlobal::get().unwrap();
//! let gns_utils = GnsUtils::new().unwrap();
//!
//! // Create a new socket, the index type [`IsCreated`] is used to determine the state of the socket.
//! // The [`GnsSocket::new`] function is only available for the [`IsCreated`] state. This is the initial state of the socket.
//! let gns_socket = GnsSocket::<IsCreated>::new().unwrap();
//!
//! // We now do a transition from [`IsCreated`] to the [`IsClient`] state. The [`GnsSocket::connect`] operation does this transition for us.
//! // Since we are now using a client socket, we have access to a different set of operations.
//! let client = gns_socket.connect(Ipv6Addr::LOCALHOST, port).unwrap();
//!
//! // Now that we initiated a connection, there is three operation we must loop over:
//! // - polling for new messages
//! // - polling for connection status change
//! // - polling for callbacks (low-level callbacks required by the underlying library).
//! // Important to know, regardless of the type of socket, whether it is in [`IsClient`] or [`IsServer`] state, theses three operations are the same.
//! // The only difference is that polling for messages and status on the client only act on the client connection, while polling for messages and status on a server yield event for all connected clients.
//!
//! loop {
//!   // Run the low-level callbacks.
//!   client.poll_callbacks();
//!
//!   // Receive a maximum of 100 messages on the client connection.
//!   // For each messages, print it's payload.
//!   let _actual_nb_of_messages_processed = client.poll_messages::<100, _>(|message| {
//!     println!(core::str::from_utf8(message.payload()).unwrap());
//!   });
//!
//!   // Don't do anything with events.
//!   // One would check the event for connection status, i.e. doing something when we are connected/disconnected from the server.
//!   let _actual_nb_of_events_processed = client.poll_event::<100, _>(|_| {
//!   });
//!
//!   // Sleep a little bit.
//!   std::thread::sleep(Duration::from_millis(10))
//! }
//! ```

use crossbeam_queue::SegQueue;
use either::Either;
pub use gns_sys as sys;
use std::{
    ffi::{c_void, CStr, CString},
    marker::PhantomData,
    mem::MaybeUninit,
    net::Ipv6Addr,
    pin::Pin,
    sync::atomic::AtomicBool,
    time::Duration,
};
use sys::*;

/// A network message number. Simple alias for documentation.
pub type GnsMessageNumber = u64;

/// Outcome of many functions from this library, basic type alias with steam [`sys::EResult`] as error.
/// If the result is [`sys::EResult::k_EResultOK`], the value can safely be wrapped, otherwise we return the error.
pub type GnsResult<T> = Result<T, EResult>;

/// Wrapper around steam [`sys::EResult`].
/// The library ensure that the wrapped value is not [`sys::EResult::k_EResultOK`].
#[repr(transparent)]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct GnsError(EResult);

impl Into<EResult> for GnsError {
    fn into(self) -> EResult {
        self.0
    }
}

impl GnsError {
    pub fn into_result(self) -> GnsResult<()> {
        self.into()
    }
}

impl From<GnsError> for GnsResult<()> {
    fn from(GnsError(result): GnsError) -> Self {
        match result {
            EResult::k_EResultOK => Ok(()),
            e => Err(e),
        }
    }
}

/// Global lock used to ensure that only one instance of [`GnsGlobal`] ever exists.
static GNS_INIT: AtomicBool = AtomicBool::new(false);

/// This is an empty type used to wrap the initialization/destruction of the low-level *GameNetworkingSockets*.
/// On construction
pub struct GnsGlobal(());

impl Drop for GnsGlobal {
    fn drop(&mut self) {
        unsafe {
            GameNetworkingSockets_Kill();
        }
        GNS_INIT.store(false, std::sync::atomic::Ordering::SeqCst)
    }
}

impl GnsGlobal {
    /// Try to acquire the [`GnsGlobal`] instance.
    /// This function will succeed only if there is no instance already created.
    /// The result might be dropped/recreated safely though.
    ///
    /// If successful, a call to [`sys::GameNetworkingSockets_Init`] has been made.
    /// Note that the drop implementation ensure that [`sys::GameNetworkingSockets_Kill`] is called.
    pub fn get() -> Result<Self, String> {
        if GNS_INIT.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        ) != Ok(false)
        {
            return Err("Only one handle of GnsGlobal must be held by a program. If the value is dropped, another handle might be created.".into());
        }
        unsafe {
            let mut error: SteamDatagramErrMsg = MaybeUninit::zeroed().assume_init();
            if !GameNetworkingSockets_Init(core::ptr::null(), &mut error) {
                GNS_INIT.store(false, std::sync::atomic::Ordering::SeqCst);
                Err(format!(
                    "{}",
                    core::str::from_utf8_unchecked(core::mem::transmute(&error[..]))
                ))
            } else {
                Ok(GnsGlobal(()))
            }
        }
    }
}

/// Simple trait used to allow for a [`GnsSocket`] state to drop itself using the parent structure `socket`.
pub trait GnsDroppable: Sized {
    fn drop(&self, socket: &GnsSocket<Self>);
}

/// Opaque wrapper around the low-level [`sys::HSteamListenSocket`].
#[repr(transparent)]
pub struct GnsListenSocket(HSteamListenSocket);

/// Opaque wrapper around the low-level [`sys::HSteamNetPollGroup`].
#[repr(transparent)]
pub struct GnsPollGroup(HSteamNetPollGroup);

/// Initial state of a [`GnsSocket`].
/// This state represent a socket that has not been used as a Server or Client implementation.
/// Consequently, the state is empty.
pub struct IsCreated;

impl GnsDroppable for IsCreated {
    fn drop(&self, _: &GnsSocket<Self>) {}
}

/// Common functions available for any [`GnsSocket`] state that is implementing it.
/// Regardless of being a client or server, a ready socket will allow us to query for connection events as well as receive messages.
pub trait IsReady: GnsDroppable {
    /// Return a reference to the connection event queue. The queue is thread-safe.
    fn queue(&self) -> &SegQueue<GnsConnectionEvent>;
    /// Poll for incoming messages. K represent the maximum number of messages we are willing to receive.
    /// Return the actual number of messsages that has been received.
    fn receive<const K: usize>(
        &self,
        gns: &GnsSocket<Self>,
        messages: &mut [GnsNetworkMessage<ToReceive>; K],
    ) -> usize;
}

/// State of a [`GnsSocket`] that has been determined to be a server, usually via the [`GnsSocket::listen`] call.
/// In this state, the socket hold the data required to accept connections and poll them for messages.
pub struct IsServer {
    /// Thread-safe FIFO queue used to read the connection status changes.
    /// Note that this structure is pinned to ensure that it's address remain the same as we are using it as connection **UserData**.
    /// This queue is meant to be passed to [`GnsSocket::on_connection_state_changed`].
    /// As long as the socket exists, this queue must exists.
    queue: Pin<Box<SegQueue<GnsConnectionEvent>>>,
    /// The low-level listen socket. Irrelevant to the user.
    listen_socket: GnsListenSocket,
    /// The low-level polling group. Irrelevant to the user.
    poll_group: GnsPollGroup,
}

impl GnsDroppable for IsServer {
    fn drop(&self, gns: &GnsSocket<Self>) {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_CloseListenSocket(gns.socket, self.listen_socket.0);
            SteamAPI_ISteamNetworkingSockets_DestroyPollGroup(gns.socket, self.poll_group.0);
        }
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
        gns: &GnsSocket<Self>,
        messages: &mut [GnsNetworkMessage<ToReceive>; K],
    ) -> usize {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_ReceiveMessagesOnPollGroup(
                gns.socket,
                self.poll_group.0,
                messages.as_mut_ptr() as _,
                K as _,
            ) as _
        }
    }
}

/// State of a [`GnsSocket`] that has been determined to be a client, usually via the [`GnsSocket::connect`] call.
/// In this state, the socket hold the data required to receive and send messages.
pub struct IsClient {
    /// Equals to [`IsServer.queue`].
    queue: Pin<Box<SegQueue<GnsConnectionEvent>>>,
    /// Actual client connection, used to receive/send messages.
    connection: GnsConnection,
}

impl GnsDroppable for IsClient {
    fn drop(&self, gns: &GnsSocket<Self>) {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_CloseConnection(
                gns.socket,
                self.connection.0,
                0,
                core::ptr::null(),
                false,
            );
        }
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
        gns: &GnsSocket<Self>,
        messages: &mut [GnsNetworkMessage<ToReceive>; K],
    ) -> usize {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_ReceiveMessagesOnConnection(
                gns.socket,
                self.connection.0,
                messages.as_mut_ptr() as _,
                K as _,
            ) as _
        }
    }
}

pub trait MayDrop {
    const MUST_DROP: bool;
}

pub struct ToSend(());

impl MayDrop for ToSend {
    const MUST_DROP: bool = false;
}

pub struct ToReceive(());

impl MayDrop for ToReceive {
    const MUST_DROP: bool = true;
}

/// Lane priority
pub type Priority = u32;
/// Lane weight
pub type Weight = u16;
/// A lane is represented by a Priority and a Weight
pub type GnsLane = (Priority, Weight);
/// A lane Id.
pub type GnsLaneId = u16;

/// Wrapper around the low-level equivalent.
/// This type is used to implements a more type-safe version of messages.
///
/// You will encounter two instances, either [`GnsNetworkMessage<ToReceive>`] or [`GnsNetworkMessage<ToSend>`].
/// The former is generated by the library and must be freed unpon handling.
/// The later is created prior to sending it via the low-level call and the low-level call itself make sure that it is freed.
#[repr(transparent)]
pub struct GnsNetworkMessage<T: MayDrop>(*mut ISteamNetworkingMessage, PhantomData<T>);

impl<T> Drop for GnsNetworkMessage<T>
where
    T: MayDrop,
{
    fn drop(&mut self) {
        if T::MUST_DROP && !self.0.is_null() {
            unsafe {
                SteamAPI_SteamNetworkingMessage_t_Release(self.0);
            }
        }
    }
}

impl<T> GnsNetworkMessage<T>
where
    T: MayDrop,
{
    /// Unsafe function you will highly unlikely use.
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
    pub fn flags(&self) -> u32 {
        unsafe { (*self.0).m_nFlags as _ }
    }

    #[inline]
    pub fn user_data(&self) -> u64 {
        unsafe { (*self.0).m_nUserData as _ }
    }

    #[inline]
    pub fn connection(&self) -> GnsConnection {
        GnsConnection(unsafe { (*self.0).m_conn })
    }

    pub fn connection_user_data(&self) -> u64 {
        unsafe { (*self.0).m_nConnUserData as _ }
    }
}

impl GnsNetworkMessage<ToSend> {
    #[inline]
    fn new(
        ptr: *mut ISteamNetworkingMessage,
        conn: GnsConnection,
        flags: i32,
        payload: &[u8],
    ) -> Self {
        GnsNetworkMessage(ptr, PhantomData)
            .set_flags(flags)
            .set_payload(payload)
            .set_connection(conn)
    }

    #[inline]
    pub fn set_connection(self, GnsConnection(conn): GnsConnection) -> Self {
        unsafe { (*self.0).m_conn = conn }
        self
    }

    #[inline]
    pub fn set_payload(self, payload: &[u8]) -> Self {
        unsafe {
            core::ptr::copy_nonoverlapping(
                payload.as_ptr(),
                (*self.0).m_pData as *mut u8,
                payload.len(),
            );
        }
        self
    }

    #[inline]
    pub fn set_lane(self, lane: u16) -> Self {
        unsafe { (*self.0).m_idxLane = lane }
        self
    }

    #[inline]
    pub fn set_flags(self, flags: i32) -> Self {
        unsafe { (*self.0).m_nFlags = flags as _ }
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
    pub fn remote_address(&self) -> Ipv6Addr {
        Ipv6Addr::from(unsafe { self.0.m_addrRemote.__bindgen_anon_1.m_ipv6 })
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
/// The drop implementation make sure that everything related to this structure is correctly freed, except the [`GnsGlobal`] and [`GnsUtils`] instances and the user has a strong guarantee that all the available operations over the socket are **safe**.
pub struct GnsSocket<'x, 'y, S: GnsDroppable> {
    global: &'x GnsGlobal,
    utils: &'y GnsUtils,
    socket: *mut ISteamNetworkingSockets,
    state: S,
}

impl<'x, 'y, S> Drop for GnsSocket<'x, 'y, S>
where
    S: GnsDroppable,
{
    fn drop(&mut self) {
        self.state.drop(&self);
    }
}

impl<'x, 'y, S> GnsSocket<'x, 'y, S>
where
    S: GnsDroppable,
{
    #[inline]
    pub unsafe fn into_inner(self) -> *mut ISteamNetworkingSockets {
        self.socket
    }

    #[inline]
    pub fn utils(&self) -> &GnsUtils {
        &self.utils
    }
}

impl<'x, 'y, S> GnsSocket<'x, 'y, S>
where
    S: GnsDroppable + IsReady,
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
        GnsError(unsafe {
            SteamAPI_ISteamNetworkingSockets_GetConnectionRealTimeStatus(
                self.socket,
                conn,
                &mut status as *mut GnsConnectionRealTimeStatus
                    as *mut SteamNetConnectionRealTimeStatus_t,
                nb_of_lanes as _,
                lanes.as_mut_ptr() as *mut GnsConnectionRealTimeLaneStatus
                    as *mut SteamNetConnectionRealTimeLaneStatus_t,
            )
        })
        .into_result()?;
        Ok((status, lanes))
    }

    #[inline]
    pub fn get_connection_info(
        &self,
        GnsConnection(conn): GnsConnection,
    ) -> Option<GnsConnectionInfo> {
        let mut info: SteamNetConnectionInfo_t = Default::default();
        if unsafe {
            SteamAPI_ISteamNetworkingSockets_GetConnectionInfo(self.socket, conn, &mut info)
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
        GnsError(unsafe {
            SteamAPI_ISteamNetworkingSockets_FlushMessagesOnConnection(self.socket, conn)
        })
        .into_result()
    }

    #[inline]
    pub fn close_connection(
        &self,
        GnsConnection(conn): GnsConnection,
        reason: u32,
        debug: &str,
        linger: bool,
    ) -> bool {
        let debug_c = CString::new(debug).expect("str; qed;");
        unsafe {
            SteamAPI_ISteamNetworkingSockets_CloseConnection(
                self.socket,
                conn,
                reason as _,
                debug_c.as_ptr(),
                linger,
            )
        }
    }

    #[inline]
    pub fn poll_messages<const K: usize, F>(&self, mut message_callback: F) -> usize
    where
        F: FnMut(&GnsNetworkMessage<ToReceive>),
    {
        // Do not implements default for networking messages as they must be allocated by the lib.
        let mut messages: [GnsNetworkMessage<ToReceive>; K] =
            unsafe { MaybeUninit::zeroed().assume_init() };
        let nb_of_messages = self.state.receive(&self, &mut messages);
        for message in messages.into_iter().take(nb_of_messages) {
            message_callback(&message);
        }
        nb_of_messages
    }

    #[inline]
    pub fn poll_event<const K: usize, F>(&self, mut event_callback: F) -> usize
    where
        F: FnMut(GnsConnectionEvent),
    {
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
    pub fn poll_callbacks(&self) {
        unsafe {
            SteamAPI_ISteamNetworkingSockets_RunCallbacks(self.socket);
        }
    }

    #[inline]
    pub fn configure_connection_lanes(
        &self,
        GnsConnection(connection): GnsConnection,
        lanes: &[GnsLane],
    ) -> GnsResult<()> {
        let (priorities, weights): (Vec<_>, Vec<_>) = lanes.iter().copied().unzip();
        GnsError(unsafe {
            SteamAPI_ISteamNetworkingSockets_ConfigureConnectionLanes(
                self.socket,
                connection,
                lanes.len() as _,
                priorities.as_ptr() as *const u32 as *const i32,
                weights.as_ptr(),
            )
        })
        .into_result()
    }

    #[inline]
    pub fn send_messages(
        &self,
        messages: Vec<GnsNetworkMessage<ToSend>>,
    ) -> Vec<Either<GnsMessageNumber, EResult>> {
        let mut result = vec![0i64; messages.len()];
        unsafe {
            SteamAPI_ISteamNetworkingSockets_SendMessages(
                self.socket,
                messages.len() as _,
                messages.as_ptr() as *const _,
                result.as_mut_ptr(),
            );
        }
        result
            .into_iter()
            .map(|value| {
                if value < 0 {
                    Either::Right(unsafe { core::mem::transmute((-value) as u32) })
                } else {
                    Either::Left(value as _)
                }
            })
            .collect()
    }
}

impl<'x, 'y> GnsSocket<'x, 'y, IsCreated> {
    /// Unsafe, C-like callback, we use the user data to pass the thread-safe event queue pointer.
    /// The library ensure that the queue is pinned in memory and valid for the lifetime of the socket using this callback.
    unsafe extern "C" fn on_connection_state_changed(
        info: &SteamNetConnectionStatusChangedCallback_t,
    ) {
        let queue =
            &*(info.m_info.m_nUserData as *const u64 as *const SegQueue<GnsConnectionEvent>);
        queue.push(GnsConnectionEvent(*info));
    }

    /// Initialize a new socket in [`IsCreated`] state.
    #[inline]
    pub fn new(global: &'x GnsGlobal, utils: &'y GnsUtils) -> Option<Self> {
        let ptr = unsafe { SteamAPI_SteamNetworkingSockets_v009() };
        if ptr.is_null() {
            None
        } else {
            Some(GnsSocket {
                global,
                utils,
                socket: ptr,
                state: IsCreated,
            })
        }
    }

    #[inline]
    fn setup_common(
        address: Ipv6Addr,
        port: u16,
        queue: &SegQueue<GnsConnectionEvent>,
    ) -> (SteamNetworkingIPAddr, [SteamNetworkingConfigValue_t; 2]) {
        let addr = SteamNetworkingIPAddr {
            __bindgen_anon_1: SteamNetworkingIPAddr__bindgen_ty_2 {
                m_ipv6: address.octets(),
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
              m_int64: queue as *const _ as i64
            }
        }];

        (addr, options)
    }

    /// Listen for incoming connections, the socket transition from [`IsCreated`] to [`IsServer`], allowing a new set of server operations.
    #[inline]
    pub fn listen(self, address: Ipv6Addr, port: u16) -> Result<GnsSocket<'x, 'y, IsServer>, ()> {
        let queue = Box::pin(SegQueue::new());
        let (addr, options) = Self::setup_common(address, port, &queue);
        let listen_socket = unsafe {
            SteamAPI_ISteamNetworkingSockets_CreateListenSocketIP(
                self.socket,
                &addr,
                options.len() as _,
                options.as_ptr(),
            )
        };
        if listen_socket == k_HSteamListenSocket_Invalid {
            Err(())
        } else {
            let poll_group =
                unsafe { SteamAPI_ISteamNetworkingSockets_CreatePollGroup(self.socket) };
            if poll_group == k_HSteamNetPollGroup_Invalid {
                Err(())
            } else {
                Ok(GnsSocket {
                    global: self.global,
                    utils: self.utils,
                    socket: self.socket,
                    state: IsServer {
                        queue,
                        listen_socket: GnsListenSocket(listen_socket),
                        poll_group: GnsPollGroup(poll_group),
                    },
                })
            }
        }
    }

    /// Connect to a remote host, the socket transition from [`IsCreated`] to [`IsClient`], allowing a new set of client operations.
    #[inline]
    pub fn connect(self, address: Ipv6Addr, port: u16) -> Result<GnsSocket<'x, 'y, IsClient>, ()> {
        let queue = Box::pin(SegQueue::new());
        let (addr, options) = Self::setup_common(address, port, &queue);
        let connection = unsafe {
            SteamAPI_ISteamNetworkingSockets_ConnectByIPAddress(
                self.socket,
                &addr,
                options.len() as _,
                options.as_ptr(),
            )
        };
        if connection == k_HSteamNetConnection_Invalid {
            Err(())
        } else {
            Ok(GnsSocket {
                global: self.global,
                utils: self.utils,
                socket: self.socket,
                state: IsClient {
                    queue,
                    connection: GnsConnection(connection),
                },
            })
        }
    }
}

impl<'x, 'y> GnsSocket<'x, 'y, IsServer> {
    /// Accept an incoming connection. This operation is available only if the socket is in the [`IsServer`] state.
    #[inline]
    pub fn accept(&self, connection: GnsConnection) -> GnsResult<()> {
        GnsError(unsafe {
            SteamAPI_ISteamNetworkingSockets_AcceptConnection(self.socket, connection.0)
        })
        .into_result()?;
        if !unsafe {
            SteamAPI_ISteamNetworkingSockets_SetConnectionPollGroup(
                self.socket,
                connection.0,
                self.state.poll_group.0,
            )
        } {
            panic!("It's impossible not to be able to set the connection poll group as both the poll group and the connection must be valid at this point.");
        }
        Ok(())
    }
}

impl<'x, 'y> GnsSocket<'x, 'y, IsClient> {
    /// Return the socket connection. This operation is available only if the socket is in the [`IsClient`] state.
    #[inline]
    pub fn connection(&self) -> GnsConnection {
        self.state.connection
    }
}

pub struct GnsUtils(*mut ISteamNetworkingUtils);

impl Drop for GnsUtils {
    fn drop(&mut self) {
        unsafe { ISteamNetworkingUtils_ISteamNetworkingUtils_destructor(self.0) }
    }
}

impl GnsUtils {
    #[inline]
    pub fn new() -> Option<Self> {
        let ptr = unsafe { SteamAPI_SteamNetworkingUtils_v003() };
        if ptr.is_null() {
            None
        } else {
            Some(GnsUtils(ptr))
        }
    }

    #[inline]
    pub fn enable_debug_output(&self, ty: ESteamNetworkingSocketsDebugOutputType) {
        unsafe extern "C" fn debug(ty: ESteamNetworkingSocketsDebugOutputType, msg: *const i8) {
            println!("{:#?}: {}", ty, CStr::from_ptr(msg).to_string_lossy());
        }
        unsafe {
            SteamAPI_ISteamNetworkingUtils_SetDebugOutputFunction(self.0, ty, Some(debug));
        }
    }

    /// Allocate a new message to be sent.
    /// This message must be sent if allocated, as the message can only be freed by the `GnsSocket::send_messages` call.
    #[inline]
    pub fn allocate_message(
        &self,
        conn: GnsConnection,
        flags: i32,
        payload: &[u8],
    ) -> GnsNetworkMessage<ToSend> {
        let message_ptr =
            unsafe { SteamAPI_ISteamNetworkingUtils_AllocateMessage(self.0, payload.len() as _) };
        GnsNetworkMessage::new(message_ptr, conn, flags, payload)
    }

    #[inline]
    pub unsafe fn into_inner(self) -> *mut ISteamNetworkingUtils {
        self.0
    }
}
