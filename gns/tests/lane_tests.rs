//! Tests for GNS connection lane configuration and quality of service
//! These tests verify the lane configuration functionality for prioritizing different types of traffic

use gns::sys::*;
use gns::{GnsGlobal, GnsLane, GnsSocket};

use std::{
    collections::HashMap,
    net::Ipv4Addr,
    sync::{Arc, Barrier, Mutex},
    thread,
    time::{Duration, Instant},
};

#[test]
fn test_connection_lane_configuration() {
    // Setup test data
    let port = 55010;
    let lane_count = 3;

    // Define three lanes with different priorities and weights
    // Lane 0: High priority (low number), low weight
    // Lane 1: Medium priority, medium weight
    // Lane 2: Low priority (high number), high weight
    let lanes: Vec<GnsLane> = vec![
        (0, 1),   // High priority, low weight
        (10, 5),  // Medium priority, medium weight
        (20, 10), // Low priority, high weight
    ];

    // Message tracking
    let server_messages = Arc::new(Mutex::new(Vec::<(usize, u16)>::new())); // (message_idx, lane)
    let lane_configured = Arc::new(Mutex::new(false));

    // Synchronization
    let server_ready = Arc::new(Barrier::new(2));
    let client_ready = Arc::new(Barrier::new(2));
    let server_done = Arc::new(Mutex::new(false));
    let client_done = Arc::new(Mutex::new(false));

    // Start server
    let server_messages_clone = server_messages.clone();
    let server_ready_clone = server_ready.clone();
    let server_done_clone = server_done.clone();

    thread::spawn(move || {
        // Initialize GNS
        let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

        // Create server socket
        let server = GnsSocket::new(gns_global.clone())
            .listen(Ipv4Addr::LOCALHOST.into(), port)
            .expect("Failed to create server socket");

        // Signal that the server is ready
        server_ready_clone.wait();

        let mut client_connection = None;

        // Main server loop
        while !*server_done_clone.lock().unwrap() {
            gns_global.poll_callbacks();

            // Process connection events
            server.poll_event::<100>(|event| {
                match (event.old_state(), event.info().state()) {
                    // New connection
                    (
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
                    ) => {
                        let result = server.accept(event.connection());
                        if result.is_ok() {
                            client_connection = Some(event.connection());
                        }
                    },

                    // Client disconnected
                    (_, ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ClosedByPeer
                    | ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ProblemDetectedLocally) => {
                        if Some(event.connection()) == client_connection {
                            client_connection = None;
                        }
                        server.close_connection(event.connection(), 0, "", false);
                    },

                    _ => {}
                }
            });

            // Process messages and record which lane they were received on
            server.poll_messages::<100>(|message| {
                let payload =
                    std::str::from_utf8(message.payload()).expect("Failed to decode message");

                // Parse message index
                if let Ok(msg_idx) = payload.trim_start_matches("Message ").parse::<usize>() {
                    let lane = message.lane();
                    server_messages_clone.lock().unwrap().push((msg_idx, lane));
                }
            });

            thread::sleep(Duration::from_millis(10));
        }
    });

    // Wait for server to start
    server_ready.wait();

    // Start client
    let lane_configured_clone = lane_configured.clone();
    let client_ready_clone = client_ready.clone();
    let client_done_clone = client_done.clone();
    let lanes_clone = lanes.clone();

    thread::spawn(move || {
        // Initialize GNS
        let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

        // Create client socket
        let client = GnsSocket::new(gns_global.clone())
            .connect(Ipv4Addr::LOCALHOST.into(), port)
            .expect("Failed to create client socket");

        // Wait for client to connect
        let mut connected = false;
        let start_time = Instant::now();
        while !connected && start_time.elapsed() < Duration::from_secs(5) {
            gns_global.poll_callbacks();

            client.poll_event::<100>(|event| {
                if event.old_state() == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting
                   && event.info().state() == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected {
                    connected = true;
                }
            });

            thread::sleep(Duration::from_millis(10));
        }

        assert!(connected, "Client failed to connect within timeout");

        // Configure connection lanes
        let result = client.configure_connection_lanes(client.connection(), &lanes_clone);

        assert!(result.is_ok(), "Failed to configure connection lanes");
        *lane_configured_clone.lock().unwrap() = true;

        // Signal that the client is ready with configured lanes
        client_ready_clone.wait();

        // Send messages on different lanes
        for i in 0..30 {
            // Determine which lane to use for this message (round-robin)
            let lane_idx = i % lane_count;

            // Create message for the appropriate lane
            let message = gns_global.utils().allocate_message(
                client.connection(),
                k_nSteamNetworkingSend_Reliable,
                format!("Message {}", i).as_bytes(),
            );

            // Set the lane
            let message = message.set_lane(lane_idx as u16);

            // Send the message
            client.send_messages(vec![message]);

            // Small delay between messages
            thread::sleep(Duration::from_millis(5));
        }

        // Wait until signaled to terminate
        while !*client_done_clone.lock().unwrap() {
            gns_global.poll_callbacks();
            thread::sleep(Duration::from_millis(10));
        }
    });

    // Wait for client to configure lanes and be ready
    client_ready.wait();

    // Verify lane configuration was successful
    assert!(
        *lane_configured.lock().unwrap(),
        "Lane configuration failed"
    );

    // Wait for messages to be received
    let start_time = Instant::now();
    let timeout = Duration::from_secs(10);

    while start_time.elapsed() < timeout {
        let msg_count = server_messages.lock().unwrap().len();
        if msg_count >= 30 {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Signal threads to terminate
    *server_done.lock().unwrap() = true;
    *client_done.lock().unwrap() = true;

    // Allow time for threads to clean up
    thread::sleep(Duration::from_millis(500));

    // Verify messages were received and which lanes they were on
    let received_messages = server_messages.lock().unwrap();

    assert!(
        received_messages.len() >= 30,
        "Not enough messages received: got {}, expected 30",
        received_messages.len()
    );

    // Verify lane assignment - count messages per lane
    let mut lane_counts = HashMap::new();
    for (_, lane) in received_messages.iter() {
        *lane_counts.entry(*lane).or_insert(0) += 1;
    }

    // There should be messages on each lane
    for lane_idx in 0..lane_count {
        let lane_id = lane_idx as u16;
        assert!(
            lane_counts.contains_key(&lane_id),
            "No messages received on lane {}",
            lane_id
        );

        let count = lane_counts[&lane_id];
        assert!(
            count >= 5,
            "Too few messages on lane {}: {}",
            lane_id,
            count
        );
    }

    // Verify that messages within each lane maintained order
    let mut lanes_msgs = vec![Vec::new(); lane_count];
    for (msg_idx, lane) in received_messages.iter() {
        lanes_msgs[*lane as usize].push(*msg_idx);
    }

    for (lane_idx, msgs) in lanes_msgs.iter().enumerate() {
        if !msgs.is_empty() {
            for i in 1..msgs.len() {
                // Messages within a lane should be in ascending order
                // This is because higher message numbers were sent later
                assert!(
                    msgs[i] > msgs[i - 1],
                    "Messages on lane {} were received out of order",
                    lane_idx
                );
            }
        }
    }
}

#[test]
fn test_get_connection_real_time_lane_status() {
    // Setup test data
    let port = 55011;

    // Number of lanes for assertions
    let lane_count = 2;

    // Track lane status readings
    let lane_status_readings = Arc::new(Mutex::new(Vec::<(
        gns::GnsConnectionRealTimeStatus,
        Vec<gns::GnsConnectionRealTimeLaneStatus>,
    )>::new()));

    // Synchronization
    let server_ready = Arc::new(Barrier::new(2));
    let client_ready = Arc::new(Barrier::new(2));
    let done = Arc::new(Mutex::new(false));

    // Client connection for the server to monitor
    let client_conn = Arc::new(Mutex::new(None));
    let client_conn_clone = client_conn.clone();

    // Start server
    let server_ready_clone = server_ready.clone();
    let done_clone = done.clone();
    let lane_status_readings_clone = lane_status_readings.clone();

    thread::spawn(move || {
        // Initialize GNS
        let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

        // Server lanes configuration
        let server_lanes: Vec<GnsLane> = vec![
            (0, 1),  // High priority, low weight
            (10, 5), // Low priority, high weight
        ];

        // Create server socket
        let server = GnsSocket::new(gns_global.clone())
            .listen(Ipv4Addr::LOCALHOST.into(), port)
            .expect("Failed to create server socket");

        // Signal that the server is ready
        server_ready_clone.wait();

        // Main server loop
        while !*done_clone.lock().unwrap() {
            gns_global.poll_callbacks();

            // Process connection events
            server.poll_event::<100>(|event| {
                match (event.old_state(), event.info().state()) {
                    // New connection
                    (
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
                    ) => {
                        let result = server.accept(event.connection());
                        if result.is_ok() {
                            *client_conn_clone.lock().unwrap() = Some(event.connection());
                        }
                    },

                    // Client disconnected
                    (_, ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ClosedByPeer
                    | ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ProblemDetectedLocally) => {
                        let mut conn = client_conn_clone.lock().unwrap();
                        if Some(event.connection()) == *conn {
                            *conn = None;
                        }
                        server.close_connection(event.connection(), 0, "", false);
                    },

                    // Client is now connected
                    (
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
                        ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected,
                    ) => {
                        // Check for lane status after connection is established
                        if let Some(conn) = *client_conn_clone.lock().unwrap() {
                            // Give the connection some time to fully establish
                            thread::sleep(Duration::from_millis(100));

                            // Configure lanes on the connection
                            let result = server.configure_connection_lanes(conn, &server_lanes);
                            assert!(result.is_ok(), "Failed to configure connection lanes");
                        }
                    },

                    _ => {}
                }
            });

            // Periodically check lane status if we have a client
            if let Some(conn) = *client_conn_clone.lock().unwrap() {
                if let Ok((status, lane_status)) =
                    server.get_connection_real_time_status(conn, lane_count as u32)
                {
                    lane_status_readings_clone
                        .lock()
                        .unwrap()
                        .push((status, lane_status));
                }
            }

            thread::sleep(Duration::from_millis(100));
        }
    });

    // Wait for server to start
    server_ready.wait();

    // Start client
    let client_ready_clone = client_ready.clone();
    let done_clone = done.clone();

    thread::spawn(move || {
        // Initialize GNS
        let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");

        // Client lanes configuration
        let client_lanes: Vec<GnsLane> = vec![
            (0, 1),  // High priority, low weight
            (10, 5), // Low priority, high weight
        ];

        // Create client socket
        let client = GnsSocket::new(gns_global.clone())
            .connect(Ipv4Addr::LOCALHOST.into(), port)
            .expect("Failed to create client socket");

        // Wait for client to connect
        let mut connected = false;
        let start_time = Instant::now();
        while !connected && start_time.elapsed() < Duration::from_secs(5) {
            gns_global.poll_callbacks();

            client.poll_event::<100>(|event| {
                if event.old_state() == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting
                   && event.info().state() == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected {
                    connected = true;
                }
            });

            thread::sleep(Duration::from_millis(10));
        }

        assert!(connected, "Client failed to connect within timeout");

        // Configure connection lanes
        let result = client.configure_connection_lanes(client.connection(), &client_lanes);

        assert!(result.is_ok(), "Failed to configure connection lanes");

        // Signal that the client is ready
        client_ready_clone.wait();

        // Send messages on different lanes to generate traffic
        for i in 0..100 {
            // Alternate between lanes
            let lane_idx = i % 2;

            // Create message for the appropriate lane
            let message = gns_global.utils().allocate_message(
                client.connection(),
                k_nSteamNetworkingSend_Reliable,
                format!("Lane Test Message {}", i).as_bytes(),
            );

            // Set the lane
            let message = message.set_lane(lane_idx as u16);

            // Send the message
            client.send_messages(vec![message]);

            // Small delay between messages
            thread::sleep(Duration::from_millis(5));
        }

        // Wait until signaled to terminate
        while !*done_clone.lock().unwrap() {
            gns_global.poll_callbacks();
            thread::sleep(Duration::from_millis(10));
        }
    });

    // Wait for client to be ready
    client_ready.wait();

    // Allow time for messages to be sent and status readings to be collected
    thread::sleep(Duration::from_secs(3));

    // Signal threads to terminate
    *done.lock().unwrap() = true;

    // Allow time for threads to clean up
    thread::sleep(Duration::from_millis(500));

    // Verify we got lane status readings
    let readings = lane_status_readings.lock().unwrap();

    assert!(!readings.is_empty(), "No lane status readings collected");

    // Check that we got the expected number of lane status values
    if let Some((_, lane_statuses)) = readings.last() {
        assert_eq!(
            lane_statuses.len(),
            lane_count,
            "Expected {} lane statuses, got {}",
            lane_count,
            lane_statuses.len()
        );
    }

    // Check some basic properties of lane status
    for (status, _) in readings.iter() {
        // Overall connection status should be connected
        assert_eq!(
            status.state(),
            ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected,
            "Connection is not in connected state"
        );
    }
}
