//! Regression tests for fixes that would otherwise silently rot:
//! - `event_queues` cleanup on socket drop (PR1.3)
//! - `GnsError::Listen` returned on port collision
//! - `GnsError::Config` returned on interior-NUL config strings

use gns::sys::*;
use gns::{GnsConfig, GnsError, GnsGlobal, GnsSocket};

use std::net::Ipv4Addr;
use std::sync::Mutex;

mod common;
use common::free_port;

/// Tests in this file all touch the shared `GnsGlobal` singleton (queue
/// counts, listen-port state, debug callback). Cargo runs `#[test]`s
/// inside one binary in parallel by default — serialize them so the
/// counts/state asserts don't race against neighbours in the same file.
static SUITE_LOCK: Mutex<()> = Mutex::new(());

/// Creating and dropping N sockets must not leak entries in
/// `GnsGlobal::event_queues`. The map's size before and after the loop
/// must be equal.
#[test]
fn test_event_queues_cleanup_on_socket_drop() {
    let _guard = SUITE_LOCK.lock().unwrap();
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let baseline = gns_global.queue_count();

    for _ in 0..16 {
        let server = GnsSocket::new(gns_global)
            .listen(Ipv4Addr::LOCALHOST.into(), free_port())
            .expect("listen");
        // Inside the scope: queue_count is baseline + 1.
        assert_eq!(gns_global.queue_count(), baseline + 1);
        drop(server);
        // Drop must purge.
        assert_eq!(gns_global.queue_count(), baseline);
    }
}

/// Listening twice on the same port must yield `GnsError::Listen`.
#[test]
fn test_listen_returns_listen_error_on_port_collision() {
    let _guard = SUITE_LOCK.lock().unwrap();
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let port = free_port();

    let _first = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), port)
        .expect("first listen");

    let second = GnsSocket::new(gns_global).listen(Ipv4Addr::LOCALHOST.into(), port);
    match second {
        Err(GnsError::Listen) => {}
        Err(other) => panic!("expected Err(Listen), got Err({:?})", other),
        Ok(_) => panic!("expected Err(Listen), got Ok"),
    }
}

/// A `GnsConfig::String` containing an interior NUL must surface as
/// `GnsError::Config("interior NUL")` rather than panicking inside
/// `CString::new`.
#[test]
fn test_config_string_interior_nul_surfaces_as_error() {
    let _guard = SUITE_LOCK.lock().unwrap();
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let result = gns_global.utils().set_global_config_value(
        ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
        GnsConfig::String("has\0null"),
    );
    match result {
        Err(GnsError::Config(msg)) => assert!(msg.contains("NUL"), "wrong message: {}", msg),
        Err(other) => panic!("expected Config error, got {:?}", other),
        Ok(()) => panic!("expected error, got Ok"),
    }
}

/// `GnsConfig::CStr` zero-allocation path: a static `c"..."` literal must
/// be accepted without going through `CString::new`.
#[test]
fn test_config_cstr_zero_alloc_path() {
    let _guard = SUITE_LOCK.lock().unwrap();
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let result = gns_global.utils().set_global_config_value(
        ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
        GnsConfig::CStr(c"stun.example.com:3478"),
    );
    assert!(
        result.is_ok(),
        "set_global_config_value via CStr failed: {:?}",
        result
    );
}
