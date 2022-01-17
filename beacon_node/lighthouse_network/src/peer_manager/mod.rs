//! Implementation of Lighthouse's peer management system.

use crate::discovery::TARGET_SUBNET_PEERS;
use crate::rpc::{GoodbyeReason, MetaData, Protocol, RPCError, RPCResponseErrorCode};
use crate::{error, metrics, Gossipsub};
use crate::{NetworkGlobals, PeerId};
use crate::{Subnet, SubnetDiscovery};
use discv5::Enr;
use hashset_delay::HashSetDelay;
use libp2p::identify::IdentifyInfo;
use parking_lot::RwLock;
use peerdb::{client::ClientKind, BanOperation, BanResult, ScoreUpdateResult};
use slog::{debug, error, warn};
use smallvec::SmallVec;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use strum::IntoEnumIterator;
use types::{EthSpec, SyncSubnetId};

pub use libp2p::core::{identity::Keypair, Multiaddr};

#[allow(clippy::mutable_key_type)] // PeerId in hashmaps are no longer permitted by clippy
pub mod peerdb;

pub use peerdb::peer_info::{
    ConnectionDirection, PeerConnectionStatus, PeerConnectionStatus::*, PeerInfo,
};
use peerdb::score::{PeerAction, ReportSource};
pub use peerdb::sync_status::{SyncInfo, SyncStatus};
use std::collections::{hash_map::Entry, HashMap};
use std::net::IpAddr;

use self::network_behaviour::ConnectionStatus;
pub mod config;
mod network_behaviour;

#[cfg(test)]
mod tests;

#[derive(Debug, PartialEq)]
pub struct Info<TSpec: EthSpec> {
    metadata: Option<MetaData<TSpec>>,
    connection_status: ConnectionStatus,
}

/// The main struct that handles peer's reputation and connection status.
pub struct PeerManager<TSpec: EthSpec> {
    /// Peer manager configuration parameters.
    config: config::Config,
    /// Known peers.
    peers: HashMap<PeerId, Info<TSpec>>,
    /// A queue of events that the `PeerManager` is waiting to produce.
    events: SmallVec<[PeerManagerEvent; 16]>,
    /// The logger associated with the `PeerManager`.
    log: slog::Logger,
}

/// The events that the `PeerManager` outputs (requests).
#[derive(Debug)]
pub enum PeerManagerEvent {
    /// A peer has dialed us.
    PeerConnectedIncoming(PeerId),
    /// A peer has been dialed.
    PeerConnectedOutgoing(PeerId),
    /// A peer has disconnected.
    PeerDisconnected(PeerId),
    /// Sends a STATUS to a peer.
    Status(PeerId),
    /// Sends a PING to a peer.
    Ping(PeerId),
    /// Request METADATA from a peer.
    MetaData(PeerId),
    /// The peer should be disconnected.
    DisconnectPeer(PeerId, GoodbyeReason),
    /// Inform the behaviour to ban this peer and associated ip addresses.
    Banned(PeerId, Vec<IpAddr>),
    /// The peer should be unbanned with the associated ip addresses.
    UnBanned(PeerId, Vec<IpAddr>),
    /// Request the behaviour to discover more peers.
    DiscoverPeers,
    /// Request the behaviour to discover peers on subnets.
    DiscoverSubnetPeers(Vec<SubnetDiscovery>),
}

impl<TSpec: EthSpec> PeerManager<TSpec> {
    // NOTE: Must be run inside a tokio executor.
    pub async fn new(
        config: config::Config,
        network_globals: Arc<NetworkGlobals<TSpec>>,
        log: &slog::Logger,
    ) -> error::Result<Self> {
        Ok(PeerManager {
            config,
            peers: HashMap::default(),
            events: SmallVec::new(),
            log: log.clone(),
        })
    }

    pub fn test(log: slog::Logger) -> Self {
        PeerManager {
            config: config::Config::default(),
            peers: HashMap::default(),
            events: SmallVec::new(),
            log,
        }
    }

    pub fn test_cfg(log: slog::Logger, config: config::Config) -> Self {
        PeerManager {
            config,
            peers: HashMap::default(),
            events: SmallVec::new(),
            log,
        }
    }

    /* Public accessible functions */

    /// The application layer wants to disconnect from a peer for a particular reason.
    ///
    /// All instant disconnections are fatal and we ban the associated peer.
    ///
    /// This will send a goodbye and disconnect the peer if it is connected or dialing.
    pub fn goodbye_peer(&mut self, peer_id: &PeerId, reason: GoodbyeReason, source: ReportSource) {}

    /// Reports a peer for some action.
    ///
    /// If the peer doesn't exist, log a warning and insert defaults.
    pub fn report_peer(
        &mut self,
        peer_id: &PeerId,
        action: PeerAction,
        source: ReportSource,
        reason: Option<GoodbyeReason>,
    ) {
    }

    /// Upon adjusting a Peer's score, there are times the peer manager must pass messages up to
    /// libp2p. This function handles the conditional logic associated with each score update
    /// result.
    fn handle_score_action(
        &mut self,
        peer_id: &PeerId,
        action: ScoreUpdateResult,
        reason: Option<GoodbyeReason>,
    ) {
    }

    /// If a peer is being banned, this handles the banning operation.
    fn handle_ban_operation(
        &mut self,
        peer_id: &PeerId,
        ban_operation: BanOperation,
        reason: Option<GoodbyeReason>,
    ) {
    }

    /// Peers that have been returned by discovery requests that are suitable for dialing are
    /// returned here.
    ///
    /// NOTE: By dialing `PeerId`s and not multiaddrs, libp2p requests the multiaddr associated
    /// with a new `PeerId` which involves a discovery routing table lookup. We could dial the
    /// multiaddr here, however this could relate to duplicate PeerId's etc. If the lookup
    /// proves resource constraining, we should switch to multiaddr dialling here.
    #[allow(clippy::mutable_key_type)]
    pub fn peers_discovered(&mut self, results: HashMap<PeerId, Option<Instant>>) -> Vec<PeerId> {
        let mut to_dial_peers = Vec::new();

        to_dial_peers
    }

    /// A STATUS message has been received from a peer. This resets the status timer.
    pub fn peer_statusd(&mut self, peer_id: &PeerId) {}

    /// Insert the sync subnet into list of long lived sync committee subnets that we need to
    /// maintain adequate number of peers for.
    pub fn add_sync_subnet(&mut self, subnet_id: SyncSubnetId, min_ttl: Instant) {}

    /// The maximum number of peers we allow to connect to us. This is `target_peers` * (1 +
    /// PEER_EXCESS_FACTOR)
    fn max_peers(&self) -> usize {
        0
    }

    /// The maximum number of peers we allow when dialing a priority peer (i.e a peer that is
    /// subscribed to subnets that our validator requires. This is `target_peers` * (1 +
    /// PEER_EXCESS_FACTOR + PRIORITY_PEER_EXCESS)
    fn max_priority_peers(&self) -> usize {
        0
    }

    /* Notifications from the Swarm */

    // A peer is being dialed.
    pub fn inject_dialing(&mut self, peer_id: &PeerId, enr: Option<Enr>) {}

    /// Reports if a peer is banned or not.
    ///
    /// This is used to determine if we should accept incoming connections.
    pub fn ban_status(&self, peer_id: &PeerId) -> BanResult {
        BanResult::BadScore
    }

    pub fn is_connected(&self, peer_id: &PeerId) -> bool {
        self.peers
            .get(peer_id)
            .map(|info| info.is_connected()) // TODO: change for allowed here
            .unwrap_or(false)
    }

    /// Reports whether the peer limit is reached in which case we stop allowing new incoming
    /// connections.
    pub fn peer_limit_reached(&self, count_dialing: bool) -> bool {
        true
    }

    /// Updates `PeerInfo` with `identify` information.
    pub fn identify(&mut self, peer_id: &PeerId, info: &IdentifyInfo) {}

    /// An error has occurred in the RPC.
    ///
    /// This adjusts a peer's score based on the error.
    pub fn handle_rpc_error(
        &mut self,
        peer_id: &PeerId,
        protocol: Protocol,
        err: &RPCError,
        direction: ConnectionDirection,
    ) {
    }

    /// A ping request has been received.
    // NOTE: The behaviour responds with a PONG automatically
    pub fn ping_request(&mut self, peer_id: &PeerId, seq: u64) {}

    /// A PONG has been returned from a peer.
    pub fn pong_response(&mut self, peer_id: &PeerId, seq: u64) {}

    /// Received a metadata response from a peer.
    pub fn meta_data_response(&mut self, peer_id: &PeerId, meta_data: MetaData<TSpec>) {}

    /// Updates the gossipsub scores for all known peers in gossipsub.
    pub(crate) fn update_gossipsub_scores(&mut self, gossipsub: &Gossipsub) {}

    // This function updates metrics for all connected peers.
    fn update_connected_peer_metrics(&self) {}

    /* Internal functions */

    /// Sets a peer as connected as long as their reputation allows it
    /// Informs if the peer was accepted
    fn inject_connect_ingoing(
        &mut self,
        peer_id: &PeerId,
        multiaddr: Multiaddr,
        enr: Option<Enr>,
    ) -> bool {
        true
    }

    /// Sets a peer as connected as long as their reputation allows it
    /// Informs if the peer was accepted
    fn inject_connect_outgoing(
        &mut self,
        peer_id: &PeerId,
        multiaddr: Multiaddr,
        enr: Option<Enr>,
    ) -> bool {
        true
    }

    /// Updates the state of the peer as disconnected.
    ///
    /// This is also called when dialing a peer fails.
    fn inject_disconnect(&mut self, peer_id: &PeerId) {}

    /// Registers a peer as connected. The `ingoing` parameter determines if the peer is being
    /// dialed or connecting to us.
    ///
    /// This is called by `connect_ingoing` and `connect_outgoing`.
    ///
    /// Informs if the peer was accepted in to the db or not.
    fn inject_peer_connection(
        &mut self,
        peer_id: &PeerId,
        connection: ConnectingType,
        enr: Option<Enr>,
    ) -> bool {
        true
    }

    // Gracefully disconnects a peer without banning them.
    fn disconnect_peer(&mut self, peer_id: PeerId, reason: GoodbyeReason) {}

    /// Run discovery query for additional sync committee peers if we fall below `TARGET_PEERS`.
    fn maintain_sync_committee_peers(&mut self) {}

    /// The Peer manager's heartbeat maintains the peer count and maintains peer reputations.
    ///
    /// It will request discovery queries if the peer count has not reached the desired number of
    /// overall peers, as well as the desired number of outbound-only peers.
    ///
    /// NOTE: Discovery will only add a new query if one isn't already queued.
    fn heartbeat(&mut self) {}

    // Update metrics related to peer scoring.
    fn update_peer_score_metrics(&self) {}
}

enum ConnectingType {
    /// We are in the process of dialing this peer.
    Dialing,
    /// A peer has dialed us.
    IngoingConnected {
        // The multiaddr the peer connected to us on.
        multiaddr: Multiaddr,
    },
    /// We have successfully dialed a peer.
    OutgoingConnected {
        /// The multiaddr we dialed to reach the peer.
        multiaddr: Multiaddr,
    },
}

impl<TSpec: EthSpec> Info<TSpec> {
    pub fn is_connected_allowed(&self) -> bool {
        matches!(
            self.connection_status,
            ConnectionStatus::Connected { reported: true, .. }
        )
    }

    fn is_connected(&self) -> bool {
        matches!(self.connection_status, ConnectionStatus::Connected { .. })
    }
}
