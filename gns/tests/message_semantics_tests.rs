//! Covers the `Payload`-driven send path:
//! - default impls (`Box<[u8]>`, `Vec<u8>`, `String`, `Arc<[u8]>`,
//!   `&'static [u8]`) and a custom impl exercise `into_raw` / `FREE_FN`,
//! - `m_nUserData` round-trips through the wrapper untouched,
//! - `send_messages` returns failed messages in `SendOutcome::Failed`.

use gns::sys::*;
use gns::{GnsConnection, GnsGlobal, GnsSocket, Payload, SendFlags, SendOutcome};

use std::{
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

mod common;
use common::free_port;

/// `m_pData` must equal `Box::into_raw`'s pointer (no internal copy).
#[test]
fn test_box_payload_is_zero_copy() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let buffer: Box<[u8]> = b"borrow-me"[..].into();
    let buffer_ptr = buffer.as_ptr();
    let buffer_len = buffer.len();

    let message =
        gns_global
            .utils()
            .allocate_message(GnsConnection::default(), SendFlags::RELIABLE, buffer);

    assert_eq!(message.payload().as_ptr(), buffer_ptr);
    assert_eq!(message.payload().len(), buffer_len);
}

/// `Arc<[u8]>::FREE_FN` must decrement the strong count on drop.
#[test]
fn test_arc_payload_is_shared_zero_copy() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let buffer: Arc<[u8]> = Arc::from(b"shared".to_vec().into_boxed_slice());
    let buffer_ptr = buffer.as_ptr();
    assert_eq!(Arc::strong_count(&buffer), 1);

    let message = gns_global.utils().allocate_message(
        GnsConnection::default(),
        SendFlags::RELIABLE,
        Arc::clone(&buffer),
    );
    assert_eq!(Arc::strong_count(&buffer), 2);
    assert_eq!(message.payload().as_ptr(), buffer_ptr);

    drop(message);
    assert_eq!(Arc::strong_count(&buffer), 1);
}

/// `&'static [u8]` is no-alloc; `FREE_FN` must be a no-op.
#[test]
fn test_static_slice_payload_is_no_op_free() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    static DATA: &[u8] = b"static-payload";

    let message =
        gns_global
            .utils()
            .allocate_message(GnsConnection::default(), SendFlags::RELIABLE, DATA);
    assert_eq!(message.payload().as_ptr(), DATA.as_ptr());
    assert_eq!(message.payload(), DATA);
}

// Third-party `Payload` impl whose free logic is expressed as ordinary
// Rust `Drop`. Demonstrates that consumers can plug in their own free
// behaviour without writing any `unsafe extern "C" fn` — the wrapper
// synthesizes the C callback from `into_raw` / `from_raw`.
static DROPS: AtomicUsize = AtomicUsize::new(0);

/// ZST whose `Drop` is the counter increment. Keeps it independent of
/// the byte buffer's reclamation so the test can verify that the
/// wrapper's free path runs `Self::Drop` exactly once per message.
struct DropCounter;
impl Drop for DropCounter {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

struct Counted {
    bytes: Box<[u8]>,
    _marker: DropCounter,
}

unsafe impl Payload for Counted {
    fn into_raw(self) -> (*mut u8, usize) {
        // Disarm `Counted`'s field destructors: ownership of `bytes` is
        // being transferred to GNS, and `_marker` should not fire its
        // counter here — it'll fire on the reconstructed value's Drop.
        let this = core::mem::ManuallyDrop::new(self);
        let bytes = unsafe { core::ptr::read(&this.bytes) };
        <Box<[u8]> as Payload>::into_raw(bytes)
    }
    unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        Counted {
            bytes: unsafe { <Box<[u8]> as Payload>::from_raw(ptr, len) },
            _marker: DropCounter,
        }
    }
}

#[test]
fn test_custom_payload_drives_free_via_drop() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let conn = GnsConnection::default();
    let start = DROPS.load(Ordering::SeqCst);

    for i in 0..32 {
        let payload = Counted {
            bytes: vec![i as u8; 8].into_boxed_slice(),
            _marker: DropCounter,
        };
        let _msg = gns_global
            .utils()
            .allocate_message(conn, SendFlags::RELIABLE, payload);
    }

    assert_eq!(DROPS.load(Ordering::SeqCst) - start, 32);
}

/// The wrapper must not consume `m_nUserData`.
#[test]
fn test_user_data_is_preserved_by_wrapper() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let message = gns_global
        .utils()
        .allocate_message(
            GnsConnection::default(),
            SendFlags::RELIABLE,
            b"user-data-test".to_vec(),
        )
        .set_user_data(0xDEAD_BEEF);

    assert_eq!(message.user_data(), 0xDEAD_BEEF);
}

/// Smoke-tests every default `Payload` impl through the unsent-drop path.
#[test]
fn test_unsent_messages_release_each_payload_kind() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let conn = GnsConnection::default();

    for _ in 0..256 {
        // Box<[u8]>
        drop(gns_global.utils().allocate_message(
            conn,
            SendFlags::RELIABLE,
            (vec![0xAA; 16]).into_boxed_slice(),
        ));
        // Vec<u8>
        drop(
            gns_global
                .utils()
                .allocate_message(conn, SendFlags::RELIABLE, vec![0xBBu8; 16]),
        );
        // String
        drop(gns_global.utils().allocate_message(
            conn,
            SendFlags::RELIABLE,
            "hello world".to_string(),
        ));
        // Arc<[u8]>
        drop(gns_global.utils().allocate_message(
            conn,
            SendFlags::RELIABLE,
            Arc::<[u8]>::from(vec![0xCCu8; 16].into_boxed_slice()),
        ));
        // &'static [u8]
        drop(
            gns_global
                .utils()
                .allocate_message(conn, SendFlags::RELIABLE, &b"static"[..]),
        );
    }
}

/// Send to a bogus connection: `send_messages` must return the original
/// message in `SendOutcome::Failed` with its payload intact.
#[test]
fn test_send_messages_returns_failed_message() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let socket = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), free_port())
        .expect("Failed to create listen socket");

    let buffer: Box<[u8]> = b"this will fail"[..].into();
    let buffer_ptr = buffer.as_ptr();
    let buffer_len = buffer.len();

    let message =
        gns_global
            .utils()
            .allocate_message(GnsConnection::default(), SendFlags::RELIABLE, buffer);

    let mut results = socket.send_messages(vec![message]);
    assert_eq!(results.len(), 1);

    match results.pop().unwrap() {
        SendOutcome::Failed(_, returned) => {
            assert_eq!(returned.payload().as_ptr(), buffer_ptr);
            assert_eq!(returned.payload().len(), buffer_len);
        }
        SendOutcome::Sent(seq) => panic!("expected failure, got Sent({})", seq),
        SendOutcome::Skipped(_) => panic!("expected failure, got Skipped"),
    }
}

/// First message fails because the handle does not match any live
/// connection; GNS sets `bCurrentConnectionFailed` and short-circuits all
/// later messages on the same handle to `result == 0`. The wrapper must
/// surface that as `Skipped`, not `Sent(0)` (per
/// `csteamnetworkingsockets.cpp:1364-1399`).
///
/// The default sentinel `GnsConnection::default()` (`0`) does NOT trigger
/// this path — it is pre-screened to `InvalidParam` before the connection
/// loop runs (csteamnetworkingsockets.cpp:1306-1313). We use a non-zero
/// fake handle to actually reach the second loop.
#[test]
fn test_send_messages_skipped_after_failure() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let socket = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), free_port())
        .expect("Failed to create listen socket");

    let fake = GnsConnection::from_raw(0xDEAD_BEEF);
    let first = gns_global
        .utils()
        .allocate_message(fake, SendFlags::RELIABLE, b"first".to_vec());
    let second = gns_global
        .utils()
        .allocate_message(fake, SendFlags::RELIABLE, b"second".to_vec());

    let results = socket.send_messages(vec![first, second]);
    assert_eq!(results.len(), 2);
    assert!(matches!(results[0], SendOutcome::Failed(_, _)));
    assert!(
        matches!(results[1], SendOutcome::Skipped(_)),
        "expected Skipped for second message on already-failed connection, got {:?}",
        match &results[1] {
            SendOutcome::Sent(s) => format!("Sent({})", s),
            SendOutcome::Failed(e, _) => format!("Failed({:?})", e),
            SendOutcome::Skipped(_) => "Skipped".into(),
        }
    );
}

/// Mixed batch: only the failing messages come back; successes are
/// consumed by GNS and reported as `SendOutcome::Sent(seq)`.
#[test]
fn test_send_messages_mixed_success_and_failure() {
    let port = free_port();

    let server_ready = Arc::new(Barrier::new(2));
    let server_done = Arc::new(Mutex::new(false));
    let server_msg_count = Arc::new(Mutex::new(0usize));

    {
        let server_ready = server_ready.clone();
        let server_done = server_done.clone();
        let server_msg_count = server_msg_count.clone();
        thread::spawn(move || {
            let gns_global = GnsGlobal::get().expect("server: Failed to initialize GNS global");
            let server = GnsSocket::new(gns_global)
                .listen(Ipv4Addr::LOCALHOST.into(), port)
                .expect("server: Failed to create listen socket");

            server_ready.wait();

            while !*server_done.lock().unwrap() {
                gns_global.poll_callbacks();
                server.poll_event::<32>(|event| {
                    if let (
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
                    ) = (event.old_state(), event.info().state())
                    {
                        let _ = server.accept(event.connection());
                    }
                });
                server.poll_messages::<32>(|_message| {
                    *server_msg_count.lock().unwrap() += 1;
                });
                thread::sleep(Duration::from_millis(10));
            }
        });
    }

    server_ready.wait();

    let gns_global = GnsGlobal::get().expect("client: Failed to initialize GNS global");
    let client = GnsSocket::new(gns_global)
        .connect(Ipv4Addr::LOCALHOST.into(), port)
        .expect("client: Failed to create client socket");

    let mut connected = false;
    let start = Instant::now();
    while !connected && start.elapsed() < Duration::from_secs(5) {
        gns_global.poll_callbacks();
        client.poll_event::<32>(|event| {
            if event.old_state()
                == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting
                && event.info().state()
                    == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected
            {
                connected = true;
            }
        });
        thread::sleep(Duration::from_millis(10));
    }
    assert!(connected, "client failed to connect");

    let bad_buffer: Box<[u8]> = b"bad"[..].into();
    let bad_ptr = bad_buffer.as_ptr();

    let good = gns_global.utils().allocate_message(
        client.connection(),
        SendFlags::RELIABLE,
        b"good".to_vec(),
    );
    let bad = gns_global.utils().allocate_message(
        GnsConnection::default(),
        SendFlags::RELIABLE,
        bad_buffer,
    );

    let results = client.send_messages(vec![good, bad]);
    assert_eq!(results.len(), 2);

    match &results[0] {
        SendOutcome::Sent(_) => {}
        SendOutcome::Failed(e, _) => panic!("good failed: {:?}", e),
        SendOutcome::Skipped(_) => panic!("good skipped"),
    }
    match &results[1] {
        SendOutcome::Failed(_, returned) => {
            assert_eq!(returned.payload().as_ptr(), bad_ptr);
        }
        SendOutcome::Sent(seq) => panic!("bad succeeded: seq={}", seq),
        SendOutcome::Skipped(_) => panic!("bad skipped"),
    }

    *server_done.lock().unwrap() = true;
    thread::sleep(Duration::from_millis(200));
}
