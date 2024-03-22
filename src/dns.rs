use std::{io::Result, net::IpAddr};

#[cfg(not(test))]
use std::net::ToSocketAddrs;

#[cfg(test)]
use mock_net::ToSocketAddrs;

pub fn resolve_domain(domain: &str) -> Result<impl Iterator<Item = IpAddr>> {
    Ok((domain, 0).to_socket_addrs()?.map(|addr| addr.ip()))
}

#[cfg(test)]
pub mod mock_net {
    use std::{io, net::SocketAddr, vec};

    use once_cell::sync::Lazy;
    use std::sync::RwLock;

    static DNS_RESULT: Lazy<
        RwLock<Box<dyn Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync>>,
    > = Lazy::new(|| RwLock::new(Box::new(|_, _| Ok(vec![]))));

    pub trait ToSocketAddrs {
        type Iter: Iterator<Item = SocketAddr>;

        fn to_socket_addrs(&self) -> io::Result<Self::Iter>;
    }

    impl ToSocketAddrs for (&str, u16) {
        type Iter = vec::IntoIter<SocketAddr>;
        fn to_socket_addrs(&self) -> io::Result<vec::IntoIter<SocketAddr>> {
            (*DNS_RESULT
                .read()
                .expect("failed to acquire read lock on DNS_RESULT"))(self.0, self.1)
            .map(|v| v.into_iter())
        }
    }

    pub fn set_socket_addrs(
        func: Box<dyn Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync>,
    ) {
        *DNS_RESULT
            .write()
            .expect("failed to acquire write lock on DNS_RESULT") = func;
    }
}
