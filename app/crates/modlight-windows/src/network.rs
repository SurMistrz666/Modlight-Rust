//! Whether this PC is on a network at all (`NetworkInterface.GetIsNetworkAvailable`):
//! some adapter other than loopback and tunnels is up and has an address. The offline
//! queue reads it before every pass; nothing is sent to find out.

use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, ERROR_SUCCESS};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_FRIENDLY_NAME,
    GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IF_TYPE_SOFTWARE_LOOPBACK, IF_TYPE_TUNNEL,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

/// True when an adapter that is not loopback is up with a unicast address. When the
/// system cannot be asked, the answer is yes: attempting and failing is a better wrong
/// guess than waiting for ever on a question nobody answered.
pub fn network_available() -> bool {
    let flags = GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER
        | GAA_FLAG_SKIP_FRIENDLY_NAME;
    // The list is built in a buffer of ours; it is aligned for the structures (u64).
    let mut size: u32 = 16 * 1024;
    for _ in 0..4 {
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let list = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `buffer` holds at least `size` bytes, suitably aligned; the system
        // writes the list into it or says how much it needs.
        let result = unsafe {
            GetAdaptersAddresses(AF_UNSPEC as u32, flags, std::ptr::null(), list, &mut size)
        };
        match result {
            ERROR_SUCCESS => {
                let mut node = list.cast_const();
                while !node.is_null() {
                    // SAFETY: `node` is an element of the list in `buffer`, alive here.
                    let adapter = unsafe { &*node };
                    if usable(adapter.OperStatus, adapter.IfType)
                        && !adapter.FirstUnicastAddress.is_null()
                    {
                        return true;
                    }
                    node = adapter.Next;
                }
                return false;
            }
            // No adapter at all.
            ERROR_NO_DATA => return false,
            // The list grew between the calls; `size` says how big it is now.
            ERROR_BUFFER_OVERFLOW => continue,
            _ => return true,
        }
    }
    true
}

/// An adapter that counts: up, and neither loopback nor a tunnel.
fn usable(status: i32, kind: u32) -> bool {
    status == IfOperStatusUp && kind != IF_TYPE_SOFTWARE_LOOPBACK && kind != IF_TYPE_TUNNEL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asking_does_not_crash() {
        // Whatever the machine's state, the call answers.
        let _ = network_available();
    }

    #[test]
    fn loopback_and_tunnels_do_not_count() {
        const ETHERNET: u32 = 6;
        assert!(usable(IfOperStatusUp, ETHERNET));
        assert!(!usable(IfOperStatusUp, IF_TYPE_SOFTWARE_LOOPBACK));
        assert!(!usable(IfOperStatusUp, IF_TYPE_TUNNEL));
        assert!(!usable(2, ETHERNET), "down");
    }
}
