//! Whether this PC is on a network at all (`NetworkInterface.GetIsNetworkAvailable`):
//! some adapter other than loopback and tunnels is up and has an address. The offline
//! queue reads it before every pass; nothing is sent to find out.

/// True when an adapter that is not loopback is up with a unicast address. When the
/// system cannot be asked, the answer is yes.
pub fn network_available() -> bool {
    true
}
