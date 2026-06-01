//! Dedicated tests for the zero-move [`GnsSocket::receive_messages_into`]
//! variant, covering the behaviours that distinguish it from the owning
//! [`GnsSocket::receive_messages`]:
//!
//! - a single caller-owned buffer is reused across many receive calls,
//! - each call yields at most `buffer.len()` messages (the runtime cap is
//!   honored — a wrong length would overflow the buffer and trip the
//!   `batch.len() <= CAP` assertion, also caught by valgrind), and
//! - dropping an iterator with unconsumed messages releases them without
//!   double-freeing or leaking, and leaves the socket usable.

use gns::sys::*;
use gns::{GnsGlobal, GnsSocket, IsClient, IsServer, MessageSlot, SendFlags};

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

mod common;
use common::free_port;

/// Establish a connected server/client pair, driven from a single thread.
fn connected_pair() -> (
    &'static GnsGlobal,
    GnsSocket<IsServer>,
    GnsSocket<IsClient>,
) {
    let gns_global = GnsGlobal::get().expect("Failed to initialize GNS global");
    let port = free_port();
    let server = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), port)
        .expect("Failed to create server socket");
    let client = GnsSocket::new(gns_global)
        .connect(Ipv4Addr::LOCALHOST.into(), port)
        .expect("Failed to create client socket");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connected = false;
    while !connected && Instant::now() < deadline {
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
        for event in client.receive_events() {
            if event.info().state()
                == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected
            {
                connected = true;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(connected, "client did not connect within the timeout");
    (gns_global, server, client)
}

/// All sent messages are delivered, in order, while a single small buffer is
/// reused for every receive call — and no call ever yields more than the
/// buffer's capacity.
#[test]
fn test_receive_messages_into_reuses_buffer_and_bounds_by_len() {
    let (gns_global, server, client) = connected_pair();

    const CAP: usize = 4;
    const N: usize = 10;

    // Send all N reliable messages up front (reliable => ordered delivery).
    for i in 0..N {
        let msg = gns_global.utils().allocate_message(
            client.connection(),
            SendFlags::RELIABLE,
            format!("msg-{i}"),
        );
        client.send_message(msg).expect("send_message failed");
    }

    // One buffer, reused for every receive call — the whole point of `_into`.
    let mut buf = [const { MessageSlot::uninit() }; CAP];
    let mut received: Vec<String> = Vec::new();

    let deadline = Instant::now() + Duration::from_secs(15);
    while received.len() < N && Instant::now() < deadline {
        gns_global.poll_callbacks();

        let batch = server
            .receive_messages_into(&mut buf)
            .expect("receive_messages_into failed");
        // The runtime length must cap each batch; a wrong count would overflow
        // the buffer and surface here (and under valgrind).
        assert!(
            batch.len() <= CAP,
            "batch of {} exceeded buffer capacity {CAP}",
            batch.len()
        );
        for message in batch {
            received.push(String::from_utf8(message.payload().to_vec()).unwrap());
        }

        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(received.len(), N, "did not receive every message");
    for (i, got) in received.iter().enumerate() {
        assert_eq!(got, &format!("msg-{i}"), "reliable order not preserved");
    }
}

/// Dropping a `receive_messages_into` iterator with unconsumed messages still
/// in it releases exactly those messages (no double free, no leak — validated
/// under valgrind) and the remaining messages are still delivered afterwards.
#[test]
fn test_receive_messages_into_releases_unconsumed_on_drop() {
    let (gns_global, server, client) = connected_pair();

    const CAP: usize = 3;
    const N: usize = 10;

    for i in 0..N {
        let msg = gns_global.utils().allocate_message(
            client.connection(),
            SendFlags::RELIABLE,
            format!("msg-{i}"),
        );
        client.send_message(msg).expect("send_message failed");
    }

    let mut buf = [const { MessageSlot::uninit() }; CAP];
    let mut dropped = 0usize;
    let mut consumed = 0usize;
    let mut did_drop = false;

    let deadline = Instant::now() + Duration::from_secs(15);
    while consumed + dropped < N && Instant::now() < deadline {
        gns_global.poll_callbacks();

        let batch = server
            .receive_messages_into(&mut buf)
            .expect("receive_messages_into failed");
        let n = batch.len();

        if n > 0 && !did_drop {
            // Drop the whole batch without consuming any of it: its `Drop`
            // must release all `n` messages.
            dropped = n;
            did_drop = true;
            drop(batch);
        } else {
            consumed += batch.count();
        }

        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(did_drop, "never received a non-empty batch to drop");
    assert!(dropped >= 1);
    // Every message not in the dropped batch is still delivered exactly once;
    // the dropped ones are gone (released, not redelivered).
    assert_eq!(
        consumed,
        N - dropped,
        "expected {} consumed after dropping {dropped}, got {consumed}",
        N - dropped
    );
}
