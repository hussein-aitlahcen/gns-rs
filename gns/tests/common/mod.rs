use std::net::UdpSocket;

/// Asks the operating system for a free UDP port on loopback.
///
/// Another process can still take the port before the caller binds it, but this
/// is good enough to keep tests off a single fixed port.
#[allow(dead_code)]
pub fn free_port() -> u16 {
    let s = UdpSocket::bind("127.0.0.1:0").expect("bind to ephemeral port");
    s.local_addr().unwrap().port()
}
