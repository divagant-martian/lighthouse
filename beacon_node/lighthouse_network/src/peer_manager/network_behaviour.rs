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
use slog::{crit, debug, error};
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
        ids: HashMap<ConnectionId, Endpoint>,
        /// If the peer has multiple connections, the time it first connected.
        since: Instant,
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
        match self.peers.entry(*peer_id) {
            Entry::Occupied(mut entry) => {
                let connection_status = &mut entry.get_mut().connection_status;
                match connection_status {
                    ConnectionStatus::Connected { ids, since } => {
                        let id_was_known = ids.insert(*connection_id, endpoint.to_endpoint());
                        debug_assert!(id_was_known.is_none());
                    }
                    ConnectionStatus::Disconnected { since } => {
                        *connection_status = ConnectionStatus::Connected {
                            ids: HashMap::from([(*connection_id, endpoint.to_endpoint())]),
                            since: Instant::now(),
                        }
                    }
                }
            }
            Entry::Vacant(entry) => {
                // Peer is new.
                entry.insert(super::Info {
                    metadata: None,
                    connection_status: ConnectionStatus::Connected {
                        ids: HashMap::from([(*connection_id, endpoint.to_endpoint())]),
                        since: Instant::now(),
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
        if let Some(ConnectionStatus::Connected { ids, since: _ }) = self
            .peers
            .get_mut(peer_id)
            .map(|info| &mut info.connection_status)
        {
            let id_was_known = ids.remove(connection_id);
            debug_assert_eq!(id_was_known, Some(endpoint.to_endpoint()));
        } else {
            #[cfg(debug_assertions)]
            panic!(
                "Connection closed for a peer in unexpected state: {}, {:?}",
                peer_id,
                self.peers.get(peer_id)
            );
            crit!(self.log, "Connection closed for a peer in unexpected state"; "peer_id" => %peer_id);
        }
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        match self.peers.entry(*peer_id) {
            Entry::Occupied(mut entry) => {
                let prev_connection_status = std::mem::replace(
                    &mut entry.get_mut().connection_status,
                    ConnectionStatus::Disconnected {
                        since: Instant::now(),
                    },
                );
                // Peer was known. It should be connected with 0 connections.
                match prev_connection_status {
                    ConnectionStatus::Connected { ids, since: _ } => debug_assert!(ids.is_empty()),
                    ConnectionStatus::Disconnected { since: _ } => {
                        #[cfg(debug_assertions)]
                        panic!(
                            "Disconnected from a disconnected peer: {}, {:?}",
                            peer_id,
                            self.peers.get(peer_id)
                        );
                        crit!(self.log, "Disconnected from already disconnected peer"; "peer_id" => %peer_id);
                    }
                }
            }
            Entry::Vacant(_) => {
                #[cfg(debug_assertions)]
                panic!("Disconnected from unknown peer: {}", peer_id,);
                crit!(self.log, "Disconnected from unknown peer"; "peer_id" => %peer_id);
            }
        }
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

    fn inject_dial_failure(
        &mut self,
        peer_id: Option<PeerId>,
        _handler: DummyProtocolsHandler,
        _error: &DialError,
    ) {
    }
}
