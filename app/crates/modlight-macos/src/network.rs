//! Whether this Mac is on a network at all (`NetworkInterface.GetIsNetworkAvailable`):
//! some interface other than loopback is up, running and has an address. The offline
//! queue reads it before every pass; nothing is sent to find out.

/// True when an interface that is not loopback is up, running and has an IPv4 or IPv6
/// address. When the system cannot be asked, the answer is yes: attempting and failing
/// is a better wrong guess than waiting for ever on a question nobody answered.
pub fn network_available() -> bool {
    #[cfg(unix)]
    {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: getifaddrs fills `list` with a linked list we free below; every node is
        // only read while the list is alive.
        if unsafe { libc::getifaddrs(&mut list) } != 0 {
            return true;
        }
        let mut found = false;
        let mut node = list;
        while !node.is_null() {
            // SAFETY: `node` is a valid element of the list getifaddrs returned.
            let entry = unsafe { &*node };
            let flags = entry.ifa_flags as libc::c_int;
            let up = flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0;
            let loopback = flags & libc::IFF_LOOPBACK != 0;
            if up && !loopback && !entry.ifa_addr.is_null() {
                // SAFETY: ifa_addr is non-null and points at a sockaddr.
                let family = unsafe { (*entry.ifa_addr).sa_family } as libc::c_int;
                if family == libc::AF_INET || family == libc::AF_INET6 {
                    found = true;
                    break;
                }
            }
            node = entry.ifa_next;
        }
        // SAFETY: `list` came from getifaddrs and is freed exactly once.
        unsafe { libc::freeifaddrs(list) };
        found
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn asking_does_not_crash() {
        // Whatever the machine's state, the call answers.
        let _ = super::network_available();
    }
}
