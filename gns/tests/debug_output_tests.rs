//! Verifies that `enable_debug_output` accepts a *capturing* closure (not just
//! a bare `fn` pointer) and actually invokes it from GNS's service thread.

use gns::sys::*;
use gns::{GnsGlobal, GnsSocket};

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod common;
use common::free_port;

#[test]
fn test_debug_output_capturing_closure() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let calls = Arc::new(AtomicUsize::new(0));
    let nonempty = Arc::new(AtomicUsize::new(0));
    {
        let calls = Arc::clone(&calls);
        let nonempty = Arc::clone(&nonempty);
        gns_global.utils().enable_debug_output(
            ESteamNetworkingSocketsDebugOutputType::k_ESteamNetworkingSocketsDebugOutputType_Everything,
            move |_ty, msg| {
                calls.fetch_add(1, Ordering::SeqCst);
                if !msg.is_empty() {
                    nonempty.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
    }

    let port = free_port();
    let server = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), port)
        .expect("Failed to create server socket");
    let client = GnsSocket::new(gns_global)
        .connect(Ipv4Addr::LOCALHOST.into(), port)
        .expect("Failed to create client socket");

    let deadline = Instant::now() + Duration::from_secs(10);
    while calls.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        gns_global.poll_callbacks();
        for event in server.receive_events() {
            if let (
                ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
                ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
            ) = (event.old_state(), event.info().state())
            {
                let _ = server.accept(event.connection());
            }
        }
        for _event in client.receive_events() {}
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        calls.load(Ordering::SeqCst) > 0,
        "the capturing debug closure was never invoked"
    );
    assert!(
        nonempty.load(Ordering::SeqCst) > 0,
        "the debug closure only ever received empty messages"
    );
}
