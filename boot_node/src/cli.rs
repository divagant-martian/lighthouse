//! Simple logic for spawning a Lighthouse BootNode.

use clap::{App, Arg, ArgGroup};

pub(crate) const IPV4_ADDR_ARG: &str = "advertised-ipv4-address";
pub(crate) const IPV6_ADDR_ARG: &str = "advertised-ipv6-address";
pub(crate) const ADVERTISED_PORT: &str = "advertised-port";

// TODO: Add DOS prevention CLI params
pub fn cli_app<'a, 'b>() -> App<'a, 'b> {
    App::new("boot_node")
        .about("Start a special Lighthouse process that only serves as a discv5 boot-node. This \
        process will *not* import blocks or perform most typical beacon node functions. Instead, it \
        will simply run the discv5 service and assist nodes on the network to discover each other. \
        This is the recommended way to provide a network boot-node since it has a reduced attack \
        surface compared to a full beacon node.")
        .settings(&[clap::AppSettings::ColoredHelp])
        .arg(
            Arg::with_name(IPV4_ADDR_ARG)
                .long(IPV4_ADDR_ARG)
                .value_name("IPV4-ADDRESS")
                .help("The external IP4 address/ DNS address to broadcast to other peers on how \
                      to reach this node. If a DNS address is provided, the enr 'ip' field is set \
                      to the IP4 address it resolves to and does not auto-update based on PONG \
                      responses in discovery.")
        )
        .arg(
            Arg::with_name(IPV6_ADDR_ARG)
                .long(IPV6_ADDR_ARG)
                .value_name("IPV6-ADDRESS")
                .help("The external IP6 address/ DNS address to broadcast to other peers on how \
                      to reach this node. If a DNS address is provided, the 'ip6' field is set to \
                      the IP6 address it resolves to and does not auto-update based on PONG \
                      responses in discovery.")
        )
        .group(
            ArgGroup::with_name("advertised-addresses")
                .required(true)
                .conflicts_with("network-dir")
                .multiple(true)
                .arg(IPV4_ADDR_ARG)
                .arg(IPV6_ADDR_ARG)
        )
        .arg(
            Arg::with_name("port")
                .long("port")
                .value_name("PORT")
                .help("The UDP port to listen on.")
                .default_value("9000")
                .takes_value(true)
        )
        .arg(
            Arg::with_name("listen-address")
                .long("listen-address")
                .value_name("ADDRESS")
                .help("The address the bootnode will listen for UDP connections.")
                .default_value("0.0.0.0")
                .takes_value(true)
        )
        .arg(
            Arg::with_name("boot-nodes")
                .long("boot-nodes")
                .allow_hyphen_values(true)
                .value_name("ENR-LIST/Multiaddr")
                .help("One or more comma-delimited base64-encoded ENR's or multiaddr strings of peers to initially add to the local routing table")
                .takes_value(true),
        )
        .arg(
            Arg::with_name(ADVERTISED_PORT)
                .long(ADVERTISED_PORT)
                .value_name("PORT")
                .help("The UDP port of the boot node's ENR. This is the port that external peers \
                      will dial to reach this boot node. Set this only if the external port \
                      differs from the listening port. If set, the 'udp' and 'udp6' ENR fields \
                      will be set depending on whether the respective 'advertised-ipv4-address' \
                      and/or 'advertised-ipv6-address' flags are given.")
                .takes_value(true)
                .conflicts_with("network-dir")
        )
        .arg(
            Arg::with_name("enable-enr-auto-update")
                .short("x")
                .long("enable-enr-auto-update")
                .help("Discovery can automatically update the node's local ENR with an external IP address and port as seen by other peers on the network. \
                This enables this feature.")
        )
        .arg(
            Arg::with_name("disable-packet-filter")
                .long("disable-packet-filter")
                .help("Disables discv5 packet filter. Useful for testing in smaller networks")
        )
        .arg(
            Arg::with_name("network-dir")
            .value_name("NETWORK_DIR")
                .long("network-dir")
                .help("The directory which contains the enr and it's assoicated private key")
                .takes_value(true)
        )
}
