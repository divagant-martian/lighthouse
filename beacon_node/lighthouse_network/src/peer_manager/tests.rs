use super::*;
use libp2p::{
    core::{connection::ConnectionId, ConnectedPoint},
    swarm::NetworkBehaviour,
};
use slog::{o, Drain};
use types::MinimalEthSpec as E;

pub fn build_log(level: slog::Level, enabled: bool) -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    if enabled {
        slog::Logger::root(drain.filter_level(level).fuse(), o!())
    } else {
        slog::Logger::root(drain.filter(|_| false).fuse(), o!())
    }
}

fn log() -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    slog::Logger::root(drain.filter_level(slog::Level::Trace).fuse(), o!())
}

/// Test that the peer manager correctly updates a peer's connection status under normal
/// circumstances.
#[test]
fn test_connection_tracking() {
    // Normal circumstances: just pretending to be informed of connections the way a swarm would
    let mut pm = PeerManager::<E>::test(log());

    let p = PeerId::random();
    let conn_id = ConnectionId::new(0);
    let address = Multiaddr::empty();
    let endpoint = ConnectedPoint::Dialer { address };

    pm.inject_connection_established(&p, &conn_id, &endpoint, None);
    assert!(pm.is_connected(&p));

    let handler = pm.new_handler();
    pm.inject_connection_closed(&p, &conn_id, &endpoint, handler);
    pm.inject_disconnected(&p);
    assert!(!pm.is_connected(&p));
}
