use std::net::UdpSocket;

/// Asks the OS for a free UDP port on loopback. Subject to a TOCTOU
/// race, but good enough to keep tests from clashing on a fixed port.
#[allow(dead_code)]
pub fn free_port() -> u16 {
    let s = UdpSocket::bind("127.0.0.1:0").expect("bind to ephemeral port");
    s.local_addr().unwrap().port()
}
