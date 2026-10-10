use std::{
    collections::HashSet,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use if_addrs::Interface;

#[derive(Debug, PartialEq, Eq)]
pub struct Ipv4Address {
    pub ip: Ipv4Addr,
    pub interface: String,
}

/// Enumerate the host's addresses without DNS, external services, or probe packets.
pub fn ipv4_addresses() -> io::Result<Vec<Ipv4Address>> {
    let interfaces = if_addrs::get_if_addrs()?;
    Ok(select_ipv4(&interfaces))
}

fn select_ipv4(interfaces: &[Interface]) -> Vec<Ipv4Address> {
    let mut addresses: Vec<_> = interfaces
        .iter()
        .filter(|interface| interface.is_oper_up())
        .filter_map(|interface| match interface.ip() {
            IpAddr::V4(ip) if usable_ipv4(ip) => Some(Ipv4Address {
                ip,
                interface: interface.name.clone(),
            }),
            _ => None,
        })
        .collect();
    // Keep physical LAN interfaces ahead of virtual bridges and tunnels. Still
    // show VPN/container addresses because they can be the intended network.
    addresses.sort_by_key(|address| {
        (
            virtual_interface(&address.interface),
            !address.ip.is_private(),
            address.ip,
            address.interface.clone(),
        )
    });
    let mut seen = HashSet::new();
    addresses.retain(|address| seen.insert(address.ip));
    addresses
}

fn usable_ipv4(ip: Ipv4Addr) -> bool {
    !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && ip.octets()[0] != 0
}

fn virtual_interface(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "docker",
        "br-",
        "veth",
        "virbr",
        "bridge",
        "tun",
        "tap",
        "utun",
        "tailscale",
        "wg",
        "vmnet",
        "vboxnet",
        "vethernet",
        "zt",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

pub fn local_url(bind: SocketAddr) -> String {
    let ip = if bind.ip().is_unspecified() {
        if bind.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        }
    } else {
        bind.ip()
    };
    format!("http://{}", SocketAddr::new(ip, bind.port()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_addrs::{IfAddr, IfOperStatus, Ifv4Addr};

    fn interface(name: &str, ip: [u8; 4], up: bool) -> Interface {
        Interface {
            name: name.into(),
            addr: IfAddr::V4(Ifv4Addr {
                ip: ip.into(),
                netmask: [255, 255, 255, 0].into(),
                prefixlen: 24,
                broadcast: None,
            }),
            index: None,
            oper_status: if up {
                IfOperStatus::Up
            } else {
                IfOperStatus::Down
            },
            is_p2p: false,
            #[cfg(windows)]
            adapter_name: name.into(),
        }
    }

    #[test]
    fn ipv4_discovery_keeps_usable_up_interfaces_and_deduplicates() {
        let interfaces = [
            interface("lo", [127, 0, 0, 1], true),
            interface("offline", [192, 168, 1, 5], false),
            interface("auto", [169, 254, 1, 5], true),
            interface("unspecified", [0, 0, 0, 0], true),
            interface("multicast", [224, 0, 0, 1], true),
            interface("broadcast", [255, 255, 255, 255], true),
            interface("eth0", [192, 168, 1, 20], true),
            interface("eth0-alias", [192, 168, 1, 20], true),
            interface("eth1", [10, 0, 0, 5], true),
        ];
        let addresses = select_ipv4(&interfaces);
        assert_eq!(addresses.len(), 2);
        assert_eq!(addresses[0].ip, Ipv4Addr::new(10, 0, 0, 5));
        assert_eq!(addresses[1].ip, Ipv4Addr::new(192, 168, 1, 20));
    }

    #[test]
    fn physical_lan_addresses_precede_virtual_bridges_without_hiding_vpn() {
        let addresses = select_ipv4(&[
            interface("docker0", [172, 17, 0, 1], true),
            interface("tailscale0", [100, 64, 1, 20], true),
            interface("en0", [192, 168, 1, 20], true),
        ]);
        assert_eq!(addresses[0].interface, "en0");
        assert_eq!(addresses.len(), 3);
    }

    #[test]
    fn browser_urls_use_real_bind_port_and_correct_address_family() {
        assert_eq!(
            local_url("0.0.0.0:4321".parse().unwrap()),
            "http://127.0.0.1:4321"
        );
        assert_eq!(
            local_url("192.168.1.20:3210".parse().unwrap()),
            "http://192.168.1.20:3210"
        );
        assert_eq!(local_url("[::]:3210".parse().unwrap()), "http://[::1]:3210");
    }
}
