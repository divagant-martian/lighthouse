/// Every configuration needed for Robert's deeds.
#[derive(Debug, Clone)]
pub struct DicksonConfig {
    /* Peer manager stuff */
    /// Ping inbound peers this often (in seconds) instead of the default `PING_INTERVAL_INBOUND`.
    pub inbound_peers_ping: Option<u64>,
    /// Ping outbound peers this often (in seconds) instead of the default `PING_INTERVAL_OUTBOUND`.
    pub outbound_peers_ping: Option<u64>,
    /// Status peers this often (in seconds) instead of the default `STATUS_INTERVAL`.
    pub status_interval: Option<u64>,
    /// Send our status after we connect to a peer
    pub status_on_connect: bool,

    /* Libp2p2 stuff */
}

impl Default for DicksonConfig {
    fn default() -> Self {
        Self{
            inbound_peers_ping: None,
            outbound_peers_ping: None,
            status_interval: None,
            status_on_connect: true,
        }
    }
}
