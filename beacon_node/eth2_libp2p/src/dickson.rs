/// Every configuration needed for Robert's deeds.
#[derive(Debug, Clone, Default)]
pub struct DicksonConfig {
    /* Peer manager stuff */
    /// See `PING_INTERVAL_INBOUND`.
    pub inbound_peers_ping: Option<u64>,
    /// See `PING_INTERVAL_OUTBOUND`.
    pub outbound_peers_ping: Option<u64>,
    /// See `STATUS_INTERVAL`.
    pub status_interval: Option<u64>,

    /* Libp2p2 stuff */
}
