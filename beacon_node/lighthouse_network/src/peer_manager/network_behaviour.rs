use std::task::{Context, Poll};

use futures::StreamExt;
use libp2p::core::connection::ConnectionId;
use libp2p::core::ConnectedPoint;
use libp2p::swarm::protocols_handler::DummyProtocolsHandler;
use libp2p::swarm::{
    DialError, NetworkBehaviour, NetworkBehaviourAction, PollParameters, ProtocolsHandler,
};
use libp2p::{Multiaddr, PeerId};
use slog::{debug, error};
use types::EthSpec;

use crate::metrics;
use crate::rpc::GoodbyeReason;
use crate::types::SyncState;

use super::peerdb::BanResult;
use super::{PeerManager, PeerManagerEvent, ReportSource};

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
        _connection_id: &ConnectionId,
        endpoint: &ConnectedPoint,
        _failed_addresses: Option<&Vec<Multiaddr>>,
    ) {
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {}

    fn inject_address_change(
        &mut self,
        _peer_id: &PeerId,
        _connection_id: &ConnectionId,
        old: &ConnectedPoint,
        new: &ConnectedPoint,
    ) {
    }

    /// A dial attempt has failed.
    ///
    /// NOTE: It can be the case that we are dialing a peer and during the dialing process the peer
    /// connects and the dial attempt later fails. To handle this, we only update the peer_db if
    /// the peer is not already connected.
    fn inject_dial_failure(
        &mut self,
        peer_id: Option<PeerId>,
        _handler: DummyProtocolsHandler,
        _error: &DialError,
    ) {
    }
}
