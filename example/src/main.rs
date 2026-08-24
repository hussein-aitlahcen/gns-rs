use gns::sys::*;
use gns::*;
use std::{
    collections::HashMap,
    net::Ipv4Addr,
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};

// This example calls `unwrap` to stay short. A real program handles every
// error instead.

fn server(port: u16) {
    // Initialize the Valve GameNetworkingSockets library and get a reference.
    let gns_global = GnsGlobal::get()
        // Handle this error in a real program.
        .unwrap();

    // Log everything. This implementation writes the log to stdout.
    gns_global.utils().enable_debug_output(
        ESteamNetworkingSocketsDebugOutputType::k_ESteamNetworkingSocketsDebugOutputType_Everything,
        |ty, message| println!("{:#?}: {}", ty, message),
    );

    // Add a fake 1000 ms ping for every client that connects.
    gns_global
        .utils()
        .set_global_config_value(
            ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_FakePacketLag_Recv,
            GnsConfig::Int32(1000),
        )
        // Handle this error in a real program.
        .unwrap();

    // The whole server state: one nickname per connected client. The server
    // generates each nickname from a counter that it increments per connection.
    let mut connected_clients = HashMap::<GnsConnection, String>::new();
    let mut nonce = 0;

    // Start the server.
    //
    // Dropping a GnsSocket cleans it up. Dropping a server closes its listen
    // socket and poll group. Dropping a client closes its connection.
    let server = GnsSocket::new(gns_global)
        .listen(Ipv4Addr::LOCALHOST.into(), port)
        // Handle this error in a real program.
        .unwrap();

    let mut last_update = Instant::now();
    loop {
        // Every 10 seconds, print the IP, ping, and byte rates of each client.
        let now = Instant::now();
        let elapsed = now - last_update;
        if elapsed.as_secs() > 10 {
            last_update = now;
            for (client, nick) in connected_clients.clone().into_iter() {
                let info = server
                    .get_connection_info(client)
                    // Handle this error in a real program.
                    .unwrap();
                let (status, _) = server
                    .get_connection_real_time_status(client, 0)
                    // Handle this error in a real program.
                    .unwrap();
                println!(
                  "== Client {:#?}\n\tIP: {:#?}\n\tPing: {:#?}\n\tOut/sec: {:#?}\n\tIn/sec: {:#?}",
                    nick,
                    info.remote_address(),
                    status.ping(),
                    status.out_bytes_per_sec(),
                    status.in_bytes_per_sec(),
                );
            }
        }

        // Run the low-level callbacks.
        gns_global.poll_callbacks();

        // Sends one message to each of the given clients. It builds the whole
        // list first, then sends it in a single call.
        let broadcast_chat = |clients: Vec<GnsConnection>, title: &str, content: &str| {
            let content: Arc<[u8]> = format!("[{}]: {}", title, content).into_bytes().into();
            let messages = clients
                .clone()
                .into_iter()
                .map(|client| {
                    gns_global.utils().allocate_message(
                        client,
                        SendFlags::RELIABLE,
                        Arc::clone(&content),
                    )
                })
                .collect::<Vec<_>>();
            // A real program checks the returned outcomes to see which messages
            // were sent.
            server.send_messages(messages);
        };

        // Process connection events.
        for event in server.receive_events() {
            match (event.old_state(), event.info().state()) {
            // A client wants to connect. Accept it.
            (
              ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
              ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
            ) => {
              let result = server.accept(event.connection());
              println!("GnsSocket<Server>: accepted new client: {:#?}.", result);
              if result.is_ok() {
                connected_clients.insert(event.connection(), nonce.to_string());
                broadcast_chat(
                  connected_clients.keys().copied().collect(),
                  "Server",
                  &format!("A new user joined us, weclome {}", nonce),
                );
                nonce += 1;
              }
              println!("GnsSocket<Server>: number of clients: {:#?}.", connected_clients.len());
            }

            // A client finished connecting. This example does nothing here, but a
            // real server could start sending it messages.
            (
              ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
              ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected,
            ) => {
            }

            (_, ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ClosedByPeer | ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ProblemDetectedLocally) => {
              // Drop the client from the list and close its connection.
              let conn = event.connection();
              println!("GnsSocket<Server>: {:#?} disconnected", conn);
              let nickname = &connected_clients[&conn];
              broadcast_chat(
                connected_clients.keys().copied().collect(),
                "Server",
                &format!("[{}] lost faith.", nickname),
              );
              connected_clients.remove(&conn);
              // Always clean up the connection. GameNetworkingSockets requires it.
              let _ = server.close_connection(conn, 0, None, false);
            }

            // Any other state change. Once a disconnected client is cleaned up, its
            // state returns to `k_ESteamNetworkingConnectionState_None`.
            (previous, current) => {
              println!("GnsSocket<Server>: {:#?} => {:#?}.", previous, current);
            }
          }
        }

        // Process incoming messages. This example handles at most 100 per
        // iteration.
        for message in server.receive_messages::<100>().into_iter().flatten() {
            let chat_message = core::str::from_utf8(message.payload())
                // Handle this error in a real program.
                .unwrap();
            println!("Boarcasting {}", chat_message);
            let sender = message.connection();
            let sender_nickname = &connected_clients[&sender];
            broadcast_chat(
                connected_clients.keys().copied().collect(),
                sender_nickname,
                chat_message,
            );
        }

        std::thread::sleep(Duration::from_millis(10))
    }
}

fn user_input() -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            // Handle this error in a real program.
            .unwrap();
        tx.send(line)
            // Handle this error in a real program.
            .unwrap();
    });
    rx
}

// The client works much like the server.
fn client(port: u16) {
    let gns_global = GnsGlobal::get()
        // Handle this error in a real program.
        .unwrap();

    println!("enable debug");
    gns_global.utils().enable_debug_output(
        ESteamNetworkingSocketsDebugOutputType::k_ESteamNetworkingSocketsDebugOutputType_Everything,
        |ty, message| println!("{:#?}: {}", ty, message),
    );

    let client = GnsSocket::new(gns_global)
        .connect(Ipv4Addr::LOCALHOST.into(), port)
        // Handle this error in a real program.
        .unwrap();

    let user_input_stream = user_input();

    'a: loop {
        gns_global.poll_callbacks();

        // Process incoming messages. This example handles at most 100 per
        // iteration.
        for message in client.receive_messages::<100>().into_iter().flatten() {
            println!(
                "(Chat) {}",
                core::str::from_utf8(message.payload())
                    // Handle this error in a real program.
                    .unwrap()
            );
        }

        let mut quit = false;
        for event in client.receive_events() {
            match (event.old_state(), event.info().state()) {
                (
                    ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
                    ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
                ) => {
                    println!("GnsSocket<Client>: connecting to server.");
                }
                (
                    ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
                    ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected,
                ) => {
                    println!("GnsSocket<Client>: connected to server.");
                }
                (_, ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ClosedByPeer | ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ProblemDetectedLocally) => {
                  // The connection closed or was lost.
                  println!("GnsSocket<Client>: disconnected.");
                  quit = true;
                }
                (previous, current) => {
                    println!("GnsSocket<Client>: {:#?} => {:#?}.", previous, current);
                }
            }
        }
        if quit {
            break 'a;
        }

        for input in user_input_stream.try_iter() {
            let input = input.trim();
            if input == "quit" {
                break 'a;
            }
            let _ = client.send_message(gns_global.utils().allocate_message(
                client.connection(),
                SendFlags::RELIABLE,
                input.as_bytes().to_vec(),
            ));
        }

        std::thread::sleep(Duration::from_millis(10))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port = 55000;
    match args.get(1).expect("server or client expected").as_str() {
        "server" => {
            server(port);
        }
        "client" => {
            client(port);
        }
        _ => panic!("either client or server"),
    }
}
