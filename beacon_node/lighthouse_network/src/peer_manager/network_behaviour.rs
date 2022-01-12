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
        ids: HashMap<ConnectionId, Endpoint>,
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
        match self.peers.entry(*peer_id) {
            Entry::Occupied(mut entry) => {
                let connection_status = &mut entry.get_mut().connection_status;
                match connection_status {
                    ConnectionStatus::Connected { ids, .. } => {
                        let id_was_known = ids.insert(*connection_id, endpoint.to_endpoint());
                        debug_assert!(id_was_known.is_none());
                    }
                    ConnectionStatus::Disconnected { since } => {
                        *connection_status = ConnectionStatus::Connected {
                            ids: HashMap::from([(*connection_id, endpoint.to_endpoint())]),
                            since: Instant::now(),
                            reported: false,
                        };
                        // Previously disconnected peer is connected now.
                        self.disconnected_count -= 1;
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
        if let Some(ConnectionStatus::Connected { ids, .. }) = self
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
        match self.peers.get_mut(peer_id) {
            Some(info) => {
                // Set the peer as disconnected.
                let prev_connection_status = std::mem::replace(
                    &mut info.connection_status,
                    ConnectionStatus::Disconnected {
                        since: Instant::now(),
                    },
                );
                // Peer was known. It should be connected with 0 connections.
                match prev_connection_status {
                    ConnectionStatus::Connected { ids, .. } => {
                        debug_assert!(ids.is_empty());
                        self.disconnected_count += 1;
                    }
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

                // Check if disconnecting this peer exceeds the max num of DC'd peers bound.
                while self.disconnected_count > self.config.max_disconnected_peers {
                    // TODO: for now simply drop the peer that has been disconnected the longest
                    let oldest_dc = self
                        .peers
                        .iter()
                        .filter_map(|(p, info)| {
                            if let ConnectionStatus::Disconnected { since } = info.connection_status
                            {
                                Some((*p, since))
                            } else {
                                None
                            }
                        })
                        .max_by_key(|(_peer_id, dc_instant)| dc_instant.elapsed().as_nanos());
                    match oldest_dc {
                        Some((p, _dc_instant)) => {
                            trace!(self.log, "Removing old disconnected peer"; "peer_id" => %p);
                            self.peers.remove(&p);
                            self.disconnected_count -= 1;
                        }
                        None => {
                            // This is a bug.
                            let truly_dc = self
                                .peers
                                .values()
                                .filter(|info| info.is_connected())
                                .count();
                            #[cfg(debug_assertions)]
                            panic!(
                                "Wrong number of disconnected peers. Registered:{}, real:{}",
                                self.disconnected_count, truly_dc
                            );
                            crit!(self.log, "Adjusting de-synced disconnected peers count"; "prev" => self.disconnected_count, "new" => truly_dc);
                            self.disconnected_count = truly_dc;
                        }
                    }
                }
            }
            None => {
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
