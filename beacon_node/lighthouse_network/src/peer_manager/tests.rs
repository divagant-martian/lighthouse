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

/// Test that the peer manager maintains the bound of disconnected peers, as well as the count of
/// disconnected peers.
#[test]
fn test_disconnected_maintainance() {
    let max_disconnected_peers = 2;
    let mut cfg = config::Config {
        max_disconnected_peers,
        ..Default::default()
    };

    let mut pm = PeerManager::<E>::test_cfg(log(), cfg);

    // connect and disconnect `max_disconnected_peers + 1` peers
    let conn_id = ConnectionId::new(0);
    let address = Multiaddr::empty();
    let endpoint = ConnectedPoint::Dialer { address };
    let mut expected_dc = 0;
    for _ in 0..=max_disconnected_peers {
        let p = PeerId::random();
        pm.inject_connection_established(&p, &conn_id, &endpoint, None);
        assert_eq!(expected_dc, pm.disconnected_count);

        let handler = pm.new_handler();
        pm.inject_connection_closed(&p, &conn_id, &endpoint, handler);
        pm.inject_disconnected(&p);
        expected_dc = max_disconnected_peers.min(expected_dc + 1);
        assert_eq!(expected_dc, pm.disconnected_count);
    }

    assert_eq!(pm.peers.len(), max_disconnected_peers);
    assert_eq!(pm.disconnected_count, max_disconnected_peers);
}
