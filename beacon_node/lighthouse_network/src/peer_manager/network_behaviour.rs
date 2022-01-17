use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::task::{Context, Poll};
use std::time::Instant;

use futures::StreamExt;
use libp2p::core::connection::ConnectionId;
use libp2p::core::{ConnectedPoint, Endpoint};
use libp2p::swarm::protocols_handler::DummyProtocolsHandler;
use libp2p::swarm::{
    DialError, NetworkBehaviour, NetworkBehaviourAction, PollParameters, ProtocolsHandler,
};
use libp2p::{Multiaddr, PeerId};
use slog::{crit, debug, error, trace};
use smallvec::SmallVec;
use types::EthSpec;

use crate::metrics;
use crate::rpc::GoodbyeReason;
use crate::types::SyncState;

use super::peerdb::BanResult;
use super::{PeerManager, PeerManagerEvent, ReportSource};

// pub struct SInstant
#[derive(Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected {
        /// Id of the peer's connection. We allow only a connection per peer (at the Swarm level).
        conn_id: ConnectionId,
        /// Direction of the peer's connection.
        direction: Endpoint,
        /// If the peer has multiple connections, the time it first connected.
        since: Instant,
        /// Whether the peer was reported as connected.
        reported: bool,
    },
    Disconnected {
        /// Time at which the peer disconnected.
        since: Instant,
    },
}

impl<TSpec: EthSpec> NetworkBehaviour for PeerManager<TSpec> {
    type ProtocolsHandler = DummyProtocolsHandler;

    type OutEvent = PeerManagerEvent;

    /* Required trait members */

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        DummyProtocolsHandler::default()
    }

    fn inject_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        _: <DummyProtocolsHandler as ProtocolsHandler>::OutEvent,
    ) {
        unreachable!("Dummy handler does not emit events")
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ProtocolsHandler>> {
        Poll::Pending
    }

    /* Overwritten trait members */

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        endpoint: &ConnectedPoint,
        _failed_addresses: Option<&Vec<Multiaddr>>,
    ) {
        // Decide if we want the peer.
        let allowed_peers = self
            .peers
            .values()
            .filter(|info| info.is_connected_allowed())
            .count();
        let reported = allowed_peers < self.config.target_peer_count;

        let conn_direction = endpoint.to_endpoint();
        match self.peers.entry(*peer_id) {
            Entry::Occupied(mut entry) => {
                let connection_status = &mut entry.get_mut().connection_status;
                match connection_status {
                    ConnectionStatus::Connected {
                        conn_id,
                        direction,
                        since,
                        reported,
                    } => {
                        // Even if the id matches this should be called only once. So either way
                        // this is problematic
                        #[cfg(debug_assertions)]
                        panic!(
                            "Connected to an already connected peer: {}. \
                               Prev conn:{:?}:{:?}. \
                               New conn:{:?}:{:?}",
                            peer_id, conn_id, direction, connection_id, endpoint
                        );
                        // Update the info
                        crit!(self.log, "Adjusting out of sync info for connected peer"; "peer_id" => %peer_id);
                        *conn_id = *connection_id;
                        *direction = conn_direction;
                        *since = Instant::now();
                    }
                    ConnectionStatus::Disconnected { since } => {
                        *connection_status = ConnectionStatus::Connected {
                            conn_id: *connection_id,
                            direction: conn_direction,
                            since: Instant::now(),
                            reported: false,
                        };
                    }
                }
            }
            Entry::Vacant(entry) => {
                // Peer is new.
                entry.insert(super::Info {
                    metadata: None,
                    connection_status: ConnectionStatus::Connected {
                        conn_id: *connection_id,
                        direction: conn_direction,
                        since: Instant::now(),
                        reported: false,
                    },
                });
            }
        }
    }

    fn inject_connection_closed(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        endpoint: &ConnectedPoint,
        _: DummyProtocolsHandler,
    ) {
        // Update the connection status of the peer.
        let conn_direction = endpoint.to_endpoint();
        match self.peers.entry(*peer_id) {
            Entry::Occupied(mut entry) => {
                let connection_status = &mut entry.get_mut().connection_status;
                match connection_status {
                    ConnectionStatus::Disconnected { since } => {
                        // Even if the id matches this should be called only once. So either way
                        // this is problematic
                        #[cfg(debug_assertions)]
                        panic!(
                            "Disconnected from an already disconnected peer: {}. \
                             Conn:{:?}:{:?}",
                            peer_id, connection_id, endpoint,
                        );
                        // Update the info
                        crit!(self.log, "Disconnected from already disconnected peer"; "peer_id" => %peer_id);
                        *since = Instant::now();
                    }
                    ConnectionStatus::Connected {
                        conn_id,
                        direction,
                        since,
                        reported,
                    } => {
                        debug_assert_eq!(conn_id, connection_id);
                        debug_assert_eq!(direction, &conn_direction);
                        *connection_status = ConnectionStatus::Disconnected {
                            since: Instant::now(),
                        };
                    }
                }
            }
            Entry::Vacant(entry) => {
                // Peer is unknown. We should know of this peer.
                #[cfg(debug_assertions)]
                panic!(
                    "Disconnected from unknown peer: {}. Conn:{:?}:{:?}",
                    peer_id, connection_id, conn_direction
                );
                entry.insert(super::Info {
                    metadata: None,
                    connection_status: ConnectionStatus::Disconnected {
                        since: Instant::now(),
                    },
                });
            }
        }

        // Check if this recently disconnected peer makes the number of disconnected peers too large.
        while self
            .peers
            .values()
            .filter(|info| !info.is_connected())
            .count()
            > self.config.max_disconnected_peers
        {
            let (oldest_dc, peer_dc_time) = self
                .peers
                .iter()
                .filter_map(|(peer_id, info)| {
                    if let ConnectionStatus::Disconnected { since } = info.connection_status {
                        Some((*peer_id, since.elapsed().as_millis()))
                    } else {
                        None
                    }
                })
                .max_by_key(|(_peer_id, dc_duration)| *dc_duration)
                .expect("Disconnected peer count > 0");

            trace!(self.log, "Purging disconnected peer"; "peer_id" => %peer_id, "dc_time_ms" => peer_dc_time );
            self.peers.remove(&oldest_dc);
        }
    }

    #[cfg(debug_assertions)]
    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        // We might not know of this peer if it was purged to keep the number of disconnected peers
        // bounded.
        if let Some(info) = self.peers.get(peer_id) {
            assert!(matches!(
                info.connection_status,
                ConnectionStatus::Disconnected { .. }
            ))
        }
    }

    #[cfg(debug_assertions)]
    fn inject_connected(&mut self, peer_id: &PeerId) {
        // A connected peer should always be known.
        let conn_status = &self
            .peers
            .get(peer_id)
            .expect("Connected peer should be known")
            .connection_status;
        assert!(matches!(conn_status, ConnectionStatus::Connected { .. }));
    }

    fn inject_address_change(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        old: &ConnectedPoint,
        new: &ConnectedPoint,
    ) {
        debug_assert_eq!(
            old.to_endpoint(),
            new.to_endpoint(),
            "Peer's connection direction has changed: {}",
            peer_id
        )
    }
}
