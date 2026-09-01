//! Tests for the introspection and utility APIs: listen socket address,
//! socket pairs, connection names, detailed status, config getters, and
//! timestamps.

use gns::sys::*;
use gns::{GnsConfig, GnsConfigValue, GnsGlobal, GnsSocket, IsClient, SendFlags};

use std::{
    net::{IpAddr, Ipv4Addr},
    thread,
    time::{Duration, Instant},
};

mod common;
use common::free_port;

/// Creates a connected socket pair for the tests below.
fn socket_pair(gns_global: &'static GnsGlobal) -> (GnsSocket<IsClient>, GnsSocket<IsClient>) {
    GnsSocket::new(gns_global)
        .socket_pair(false)
        .expect("Failed to create socket pair")
}

#[test]
fn test_listen_socket_address_reports_bound_address() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let fixed_port = free_port();
    let server = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), fixed_port)
        .expect("Failed to create server socket");
    let (addr, port) = server
        .get_listen_socket_address()
        .expect("Failed to get listen socket address");
    assert_eq!(port, fixed_port);
    assert_eq!(addr, IpAddr::from(Ipv4Addr::LOCALHOST));
}

#[test]
fn test_socket_pair_send_receive() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let (a, b) = socket_pair(gns_global);

    let before_send = gns_global.utils().local_timestamp();
    let message =
        gns_global
            .utils()
            .allocate_message(a.connection(), SendFlags::RELIABLE, "ping over pair");
    a.send_message(message).expect("Failed to send message");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while received.is_empty() && Instant::now() < deadline {
        gns_global.poll_callbacks();
        for message in b.receive_messages::<10>().expect("receive failed") {
            assert!(
                message.time_received() >= before_send,
                "message timestamp predates the send"
            );
            received.push(String::from_utf8(message.payload().to_vec()).unwrap());
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(received, vec!["ping over pair".to_string()]);
}

#[test]
fn test_detailed_connection_status() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let (a, _b) = socket_pair(gns_global);

    let status = a
        .get_detailed_connection_status(a.connection())
        .expect("Failed to get detailed connection status");
    assert!(!status.is_empty(), "expected a non-empty status dump");

    // An invalid handle yields None instead of an error string.
    assert!(a
        .get_detailed_connection_status(gns::GnsConnection::default())
        .is_none());
}

#[test]
fn test_connection_name_roundtrip() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let (a, _b) = socket_pair(gns_global);

    a.set_connection_name(a.connection(), "pair-side-a")
        .expect("Failed to set connection name");
    let name = a
        .get_connection_name(a.connection())
        .expect("Failed to get connection name");
    assert_eq!(name, "pair-side-a");

    // An interior NUL byte is rejected instead of truncating the name.
    assert!(a.set_connection_name(a.connection(), "bad\0name").is_err());
}

#[test]
fn test_global_config_value_roundtrip() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let utils = gns_global.utils();

    utils
        .set_global_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_TimeoutConnected,
            GnsConfig::Int32(12345),
        )
        .expect("Failed to set config value");
    assert_eq!(
        utils
            .get_global_config_value(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_TimeoutConnected,
            )
            .expect("Failed to get config value"),
        GnsConfigValue::Int32(12345)
    );

    utils
        .set_global_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_FakePacketLoss_Recv,
            GnsConfig::Float(1.5),
        )
        .expect("Failed to set config value");
    assert_eq!(
        utils
            .get_global_config_value(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_FakePacketLoss_Recv,
            )
            .expect("Failed to get config value"),
        GnsConfigValue::Float(1.5)
    );

    utils
        .set_global_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            GnsConfig::String("stun.example.org:3478"),
        )
        .expect("Failed to set config value");
    assert_eq!(
        utils
            .get_global_config_value(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            )
            .expect("Failed to get config value"),
        GnsConfigValue::String("stun.example.org:3478".to_string())
    );
}

#[test]
fn test_connection_config_value_roundtrip() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let utils = gns_global.utils();
    let (a, _b) = socket_pair(gns_global);

    utils
        .set_connection_config_value(
            a.connection(),
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_SendBufferSize,
            GnsConfig::Int32(1024 * 1024),
        )
        .expect("Failed to set connection config value");
    assert_eq!(
        utils
            .get_connection_config_value(
                a.connection(),
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_SendBufferSize,
            )
            .expect("Failed to get connection config value"),
        GnsConfigValue::Int32(1024 * 1024)
    );
}

#[test]
fn test_listen_socket_config_value_roundtrip() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let server = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), free_port())
        .expect("Failed to create server socket");

    server
        .set_listen_socket_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_SendBufferSize,
            GnsConfig::Int32(2 * 1024 * 1024),
        )
        .expect("Failed to set listen socket config value");
    assert_eq!(
        server
            .get_listen_socket_config_value(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_SendBufferSize,
            )
            .expect("Failed to get listen socket config value"),
        GnsConfigValue::Int32(2 * 1024 * 1024)
    );
}

#[test]
fn test_config_value_info() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    let (name, data_type, scope) = gns_global
        .utils()
        .get_config_value_info(ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_TimeoutInitial)
        .expect("Failed to get config value info");
    assert_eq!(name, "TimeoutInitial");
    assert_eq!(
        data_type,
        ESteamNetworkingConfigDataType::k_ESteamNetworkingConfig_Int32
    );
    assert_eq!(
        scope,
        ESteamNetworkingConfigScope::k_ESteamNetworkingConfig_Connection
    );

    assert!(gns_global
        .utils()
        .get_config_value_info(ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_Invalid)
        .is_none());
}

#[test]
fn test_global_config_cstr_roundtrip() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let utils = gns_global.utils();

    // The TURN list, not the STUN list: another test in this binary owns the
    // STUN list, and tests in one binary run in parallel against the shared
    // global configuration.
    let value = c"turn.example.org:3478";
    utils
        .set_global_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_TURN_ServerList,
            GnsConfig::CStr(value),
        )
        .expect("Failed to set config value");
    assert_eq!(
        utils
            .get_global_config_value(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_TURN_ServerList,
            )
            .expect("Failed to get config value"),
        GnsConfigValue::String("turn.example.org:3478".to_string())
    );
}

// No Float roundtrip at connection or listen-socket scope: every Float config
// in this GameNetworkingSockets version is global-scope only (the fake
// loss/jitter percentages), so the global test above is the only place a
// Float value can be set and read back.
#[test]
fn test_connection_config_string_and_cstr_roundtrip() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let utils = gns_global.utils();
    let (a, _b) = socket_pair(gns_global);

    utils
        .set_connection_config_value(
            a.connection(),
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            GnsConfig::String("stun.conn.example.org:3478"),
        )
        .expect("Failed to set string config value");
    assert_eq!(
        utils
            .get_connection_config_value(
                a.connection(),
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            )
            .expect("Failed to get string config value"),
        GnsConfigValue::String("stun.conn.example.org:3478".to_string())
    );

    let cstr = c"stun.cstr.example.org:3478";
    utils
        .set_connection_config_value(
            a.connection(),
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            GnsConfig::CStr(cstr),
        )
        .expect("Failed to set cstr config value");
    assert_eq!(
        utils
            .get_connection_config_value(
                a.connection(),
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            )
            .expect("Failed to get cstr config value"),
        GnsConfigValue::String("stun.cstr.example.org:3478".to_string())
    );
}

#[test]
fn test_listen_socket_config_string_roundtrip() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let server = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), free_port())
        .expect("Failed to create server socket");

    // A listen socket stores connection options as defaults for accepted
    // connections, so connection-scoped values are settable here. The string
    // goes through the generic `SetConfigValue` entry point and its
    // pointer-to-pointer convention.
    server
        .set_listen_socket_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            GnsConfig::String("stun.listen.example.org:3478"),
        )
        .expect("Failed to set string config value");
    assert_eq!(
        server
            .get_listen_socket_config_value(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            )
            .expect("Failed to get string config value"),
        GnsConfigValue::String("stun.listen.example.org:3478".to_string())
    );

    let cstr = c"stun.listen-cstr.example.org:3478";
    server
        .set_listen_socket_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            GnsConfig::CStr(cstr),
        )
        .expect("Failed to set cstr config value");
    assert_eq!(
        server
            .get_listen_socket_config_value(
                ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
            )
            .expect("Failed to get cstr config value"),
        GnsConfigValue::String("stun.listen-cstr.example.org:3478".to_string())
    );
}

#[test]
fn test_connection_config_int64_and_ptr_read() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let utils = gns_global.utils();
    let (a, _b) = socket_pair(gns_global);

    // The wrapper stores its event queue ID in the connection user data, so
    // reading it back exercises the Int64 path with a value that is known to
    // be set.
    let user_data = utils
        .get_connection_config_value(
            a.connection(),
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_ConnectionUserData,
        )
        .expect("Failed to get connection user data");
    assert!(
        matches!(user_data, GnsConfigValue::Int64(id) if id >= 0),
        "expected a non-negative Int64 queue ID, got {user_data:?}"
    );

    // `socket_pair` installs the connection-state callback through the Ptr
    // path, so reading it back exercises the Ptr path and confirms the
    // callback actually landed.
    let callback = utils
        .get_connection_config_value(
            a.connection(),
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_Callback_ConnectionStatusChanged,
        )
        .expect("Failed to get connection callback");
    assert!(
        matches!(callback, GnsConfigValue::Ptr(p) if !p.is_null()),
        "expected a non-null callback pointer, got {callback:?}"
    );
}

#[test]
fn test_local_timestamp() {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let utils = gns_global.utils();

    let first = utils.local_timestamp();
    assert!(first > 0, "expected a positive timestamp, got {first}");
    thread::sleep(Duration::from_millis(5));
    let second = utils.local_timestamp();
    assert!(
        second > first,
        "expected the timestamp to advance: {first} -> {second}"
    );
}
