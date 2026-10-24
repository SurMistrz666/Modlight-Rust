//! Whether this Mac is on a network at all (`NetworkInterface.GetIsNetworkAvailable`):
//! some interface other than loopback is up, running and has an address. The offline
//! queue reads it before every pass; nothing is sent to find out.

#[cfg(unix)]
use std::net::IpAddr;

/// One address of an interface, as `getifaddrs` lists it: the interface's flags and the
/// address, when it is an IPv4 or IPv6 one.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceAddress {
    pub flags: libc::c_int,
    pub address: Option<IpAddr>,
}

/// Whether one of these addresses is a network to be on. A link-local IPv6 address
/// (`fe80::/10`) does not count: every Mac keeps interfaces up with only such an address
/// when it is on no network at all (the system's tunnels `utun0`..., AirDrop's `awdl0`
/// and `llw0`), and Windows leaves tunnels out of the same question.
#[cfg(unix)]
pub fn available(addresses: &[InterfaceAddress]) -> bool {
    addresses.iter().any(|entry| {
        let flags = entry.flags;
        let up = flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0;
        let loopback = flags & libc::IFF_LOOPBACK != 0;
        let reaches = match entry.address {
            Some(IpAddr::V4(v4)) => !v4.is_unspecified() && !v4.is_loopback(),
            Some(IpAddr::V6(v6)) => {
                !v6.is_unspecified() && !v6.is_loopback() && !v6.is_unicast_link_local()
            }
            None => false,
        };
        up && !loopback && reaches
    })
}

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
        let mut addresses = Vec::new();
        let mut node = list;
        while !node.is_null() {
            // SAFETY: `node` is a valid element of the list getifaddrs returned.
            let entry = unsafe { &*node };
            addresses.push(InterfaceAddress {
                flags: entry.ifa_flags as libc::c_int,
                // SAFETY: ifa_addr is null or points at a sockaddr of the family it names.
                address: unsafe { ip_address(entry.ifa_addr) },
            });
            node = entry.ifa_next;
        }
        // SAFETY: `list` came from getifaddrs and is freed exactly once.
        unsafe { libc::freeifaddrs(list) };
        available(&addresses)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The IPv4 or IPv6 address a `sockaddr` holds.
///
/// # Safety
/// `address` is null or points at a `sockaddr` as long as its family says.
#[cfg(unix)]
unsafe fn ip_address(address: *const libc::sockaddr) -> Option<IpAddr> {
    if address.is_null() {
        return None;
    }
    // SAFETY: the caller's promise.
    let family = unsafe { (*address).sa_family } as libc::c_int;
    if family == libc::AF_INET {
        // SAFETY: an AF_INET sockaddr is a sockaddr_in.
        let v4 = unsafe { &*(address as *const libc::sockaddr_in) };
        Some(IpAddr::from(u32::from_be(v4.sin_addr.s_addr).to_be_bytes()))
    } else if family == libc::AF_INET6 {
        // SAFETY: an AF_INET6 sockaddr is a sockaddr_in6.
        let v6 = unsafe { &*(address as *const libc::sockaddr_in6) };
        Some(IpAddr::from(v6.sin6_addr.s6_addr))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn asking_does_not_crash() {
        // Whatever the machine's state, the call answers.
        let _ = super::network_available();
    }

    #[cfg(unix)]
    mod addresses {
        use super::super::*;

        const UP: libc::c_int = libc::IFF_UP | libc::IFF_RUNNING;

        fn entry(flags: libc::c_int, address: &str) -> InterfaceAddress {
            InterfaceAddress {
                flags,
                address: address.parse().ok(),
            }
        }

        /// A Mac with Wi-Fi and Ethernet off still has interfaces that are up, running
        /// and addressed: the system's tunnels (`utun0`...), AirDrop's `awdl0` and
        /// `llw0`. They carry only link-local IPv6 addresses (`fe80::/10`), which reach
        /// no network (upstream does not count tunnels either).
        #[test]
        fn link_local_addresses_are_no_network() {
            let offline = [
                entry(UP | libc::IFF_LOOPBACK, "127.0.0.1"),
                entry(UP | libc::IFF_LOOPBACK, "::1"),
                entry(UP | libc::IFF_LOOPBACK, "fe80::1"),
                entry(UP | libc::IFF_POINTOPOINT, "fe80::1294:b263:1332:3533"),
                entry(UP | libc::IFF_BROADCAST, "fe80::4bc:9eff:fe5f:1e09"),
                entry(libc::IFF_UP | libc::IFF_BROADCAST, "192.168.1.20"),
                entry(UP | libc::IFF_BROADCAST, ""),
            ];
            assert!(!available(&offline));

            let with = |address: &str| {
                let mut list = offline.to_vec();
                list.push(entry(UP | libc::IFF_BROADCAST, address));
                available(&list)
            };
            assert!(with("192.168.1.20"));
            assert!(with("2a02:a311:4040::1"));
            // A self-assigned IPv4 address: the adapter is up (Windows says so too).
            assert!(with("169.254.12.7"));
        }
    }
}
