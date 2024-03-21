use std::{
    io::Result,
    net::{IpAddr, ToSocketAddrs},
};

pub fn resolve_domain(domain: &str) -> Result<impl Iterator<Item = IpAddr>> {
    Ok((domain, 0).to_socket_addrs()?.map(|addr| addr.ip()))
}
