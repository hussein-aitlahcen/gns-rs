//! Tests for GNS configuration and debug output
//! These tests verify the configuration and debug capabilities of the GNS library

use gns::sys::*;
use gns::{GnsConfig, GnsGlobal, GnsSocket};

use std::net::Ipv4Addr;

#[test]
fn test_global_config_values() {
    // Initialize GNS
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let utils = gns_global.utils();

    // Test setting float configuration value
    let result = utils.set_global_config_value(
        ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_TimeoutInitial,
        GnsConfig::Float(15.0),
    );
    assert!(result.is_ok(), "Failed to set float config value");

    // Test setting integer configuration value
    let result = utils.set_global_config_value(
        ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_FakePacketLag_Send,
        GnsConfig::Int32(100),
    );
    assert!(result.is_ok(), "Failed to set integer config value");

    // Test setting string configuration value
    let result = utils.set_global_config_value(
        ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_STUN_ServerList,
        GnsConfig::String("stun1.example.com:3478"),
    );
    assert!(result.is_ok(), "Failed to set string config value");
}

#[test]
fn test_message_flags() {
    // Initialize GNS
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    // Allocate a message with reliable flag
    let connection = gns::GnsConnection::default();
    let message = gns_global.utils().allocate_message(
        connection,
        k_nSteamNetworkingSend_Reliable,
        b"Test message",
    );

    // Verify flag is set correctly
    assert_eq!(
        message.flags() & k_nSteamNetworkingSend_Reliable as i32,
        k_nSteamNetworkingSend_Reliable as i32,
        "Reliable flag not set correctly"
    );

    // Allocate a message with unreliable flag
    let message = gns_global.utils().allocate_message(
        connection,
        k_nSteamNetworkingSend_Unreliable,
        b"Test message",
    );

    // Verify flag is set correctly
    assert_eq!(
        message.flags() & k_nSteamNetworkingSend_Unreliable as i32,
        k_nSteamNetworkingSend_Unreliable as i32,
        "Unreliable flag not set correctly"
    );

    // Test setting user data
    let user_data = 12345;
    let message = gns_global
        .utils()
        .allocate_message(connection, k_nSteamNetworkingSend_Reliable, b"Test message")
        .set_user_data(user_data);

    // Verify user data is set correctly
    assert_eq!(
        message.user_data(),
        user_data,
        "User data not set correctly"
    );
}

#[test]
fn test_connection_info() {
    // Initialize GNS
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

    // Create a connection object
    let conn = gns::GnsConnection::default();

    // Just check that the get_connection_info function exists and doesn't crash
    // We expect it to return None for a default connection
    let info = GnsSocket::new(gns_global.clone())
        .listen(Ipv4Addr::LOCALHOST.into(), 59999)
        .expect("Failed to create server socket")
        .get_connection_info(conn);

    // It should return None for an invalid connection
    assert!(info.is_none(), "Expected None for invalid connection info");
}
