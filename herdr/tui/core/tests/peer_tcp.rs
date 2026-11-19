//! A TCP sender named by the process holding its port, alone in a binary of its own: a socket of this process that a
//! child of another test inherited would be held by that child too, and name it.

use cockpit_core::peer::{Sender, current, set_tcp};

#[test]
fn a_tcp_peer_is_named_by_its_process_once_and_an_unnamed_one_is_unknown() {
    let hub = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let at = hub.local_addr().unwrap().port();
    let script = format!("exec 3<>/dev/tcp/127.0.0.1/{at}; exec sleep 5");
    let mut child = std::process::Command::new("bash").args(["-c", &script]).spawn().unwrap();
    let (_served, from) = hub.accept().unwrap();
    set_tcp(from.port(), at);
    assert_eq!(current(), Sender::Process(child.id()));
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(current(), Sender::Process(child.id()), "looked up once for the connection");
    // This process at both ends: the lookup leaves the hub's own pid out, so nothing is named.
    let own = std::net::TcpStream::connect(("127.0.0.1", at)).unwrap();
    let (_served, from) = hub.accept().unwrap();
    assert_eq!(from.port(), own.local_addr().unwrap().port());
    set_tcp(from.port(), at);
    assert_eq!(current(), Sender::Unknown, "never taken for a person");
}
