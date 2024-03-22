use http::HeaderValue;
use std::{net::IpAddr, str::FromStr, time::Duration};
use tonic::transport::{ClientTlsConfig, Endpoint, Uri};
use url::{Host, Url};

pub struct EndpointBuilder {
    url: Url,
    origin: Option<Uri>,
    user_agent: Option<HeaderValue>,
    concurrency_limit: Option<usize>,
    rate_limit: Option<(u64, Duration)>,
    timeout: Option<Duration>,
    #[cfg(feature = "tls")]
    tls_config: Option<ClientTlsConfig>,
    buffer_size: Option<usize>,
    init_stream_window_size: Option<u32>,
    init_connection_window_size: Option<u32>,
    tcp_keepalive: Option<Duration>,
    tcp_nodelay: Option<bool>,
    http2_keep_alive_interval: Option<Duration>,
    http2_keep_alive_timeout: Option<Duration>,
    http2_keep_alive_while_idle: Option<bool>,
    connect_timeout: Option<Duration>,
    http2_adaptive_window: Option<bool>,
    // executor: dyn Executor<Pin<Box<dyn Future<Output = ()> + Send>>> + Send + Sync + 'static,
}

impl EndpointBuilder {
    pub fn new(url: impl Into<Url>) -> Result<Self, ()> {
        let url: Url = url.into();

        // Check if URL contains hostname that can be resolved with DNS
        match url.host() {
            Some(host) => match host {
                Host::Domain(_) => {}
                Host::Ipv4(_) => todo!(),
                Host::Ipv6(_) => todo!(),
            },
            None => todo!(),
        }

        // Check if hostname in URL can be substituted by IP address
        if url.cannot_be_a_base() {
            todo!()
        }

        // Check if tonic Uri can be build from Url
        if Uri::from_str(url.as_str()).is_err() {
            todo!()
        }

        Ok(Self {
            url,
            origin: None,
            user_agent: None,
            timeout: None,
            concurrency_limit: None,
            rate_limit: None,
            #[cfg(feature = "tls")]
            tls_config: None,
            buffer_size: None,
            init_stream_window_size: None,
            init_connection_window_size: None,
            tcp_keepalive: None,
            tcp_nodelay: None,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: None,
            http2_keep_alive_while_idle: None,
            connect_timeout: None,
            http2_adaptive_window: None,
        })
    }

    pub fn origin(self, origin: Uri) -> Self {
        Self {
            origin: Some(origin),
            ..self
        }
    }

    pub fn user_agent(self, user_agent: impl TryInto<HeaderValue>) -> Self {
        Self {
            user_agent: Some(
                user_agent
                    .try_into()
                    .map_err(|_| "fubar")
                    .expect("header value"),
            ),
            ..self
        }
    }

    pub fn timeout(self, dur: Duration) -> Self {
        Self {
            timeout: Some(dur),
            ..self
        }
    }

    pub fn connect_timeout(self, dur: Duration) -> Self {
        Self {
            connect_timeout: Some(dur),
            ..self
        }
    }

    pub fn tcp_keepalive(self, tcp_keepalive: Option<Duration>) -> Self {
        Self {
            tcp_keepalive,
            ..self
        }
    }

    pub fn concurrency_limit(self, limit: usize) -> Self {
        Self {
            concurrency_limit: Some(limit),
            ..self
        }
    }

    pub fn rate_limit(self, limit: u64, duration: Duration) -> Self {
        Self {
            rate_limit: Some((limit, duration)),
            ..self
        }
    }

    pub fn initial_stream_window_size(self, sz: impl Into<Option<u32>>) -> Self {
        Self {
            init_stream_window_size: sz.into(),
            ..self
        }
    }

    pub fn initial_connection_window_size(self, sz: impl Into<Option<u32>>) -> Self {
        Self {
            init_connection_window_size: sz.into(),
            ..self
        }
    }

    pub fn buffer_size(self, sz: impl Into<Option<usize>>) -> Self {
        Self {
            buffer_size: sz.into(),
            ..self
        }
    }

    #[cfg(feature = "tls")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
    pub fn tls_config(self, tls_config: ClientTlsConfig) -> Self {
        //TODO: check conversion here
        Self {
            tls_config: Some(tls_config),
            ..self
        }
    }

    pub fn tcp_nodelay(self, enabled: bool) -> Self {
        Self {
            tcp_nodelay: Some(enabled),
            ..self
        }
    }

    pub fn http2_keep_alive_interval(self, interval: Duration) -> Self {
        Self {
            http2_keep_alive_interval: Some(interval),
            ..self
        }
    }

    pub fn keep_alive_timeout(self, duration: Duration) -> Self {
        Self {
            http2_keep_alive_timeout: Some(duration),
            ..self
        }
    }

    pub fn keep_alive_while_idle(self, enabled: bool) -> Self {
        Self {
            http2_keep_alive_while_idle: Some(enabled),
            ..self
        }
    }

    pub fn http2_adaptive_window(self, enabled: bool) -> Self {
        Self {
            http2_adaptive_window: Some(enabled),
            ..self
        }
    }

    pub fn build(&self, ip_address: impl Into<IpAddr>) -> Endpoint {
        let mut endpoint = Endpoint::from(self.build_uri(ip_address.into()));

        if let Some(origin) = self.origin.clone() {
            endpoint = endpoint.origin(origin);
        }

        if let Some(user_agent) = self.user_agent.clone() {
            // user_agent is already of the correct type so this will never
            // return an error.
            endpoint = endpoint.user_agent(user_agent).unwrap();
        }

        if let Some(timeout) = self.timeout {
            endpoint = endpoint.timeout(timeout)
        }

        if let Some(connect_timeout) = self.connect_timeout {
            endpoint = endpoint.connect_timeout(connect_timeout)
        }

        endpoint = endpoint.tcp_keepalive(self.tcp_keepalive);

        if let Some(limit) = self.concurrency_limit {
            endpoint = endpoint.concurrency_limit(limit)
        }

        if let Some((limit, duration)) = self.rate_limit {
            endpoint = endpoint.rate_limit(limit, duration);
        }

        if let Some(sz) = self.init_stream_window_size {
            endpoint = endpoint.initial_stream_window_size(sz);
        }

        if let Some(sz) = self.init_connection_window_size {
            endpoint = endpoint.initial_connection_window_size(sz);
        }

        endpoint = endpoint.buffer_size(self.buffer_size);

        if let Some(tls_config) = self.tls_config.clone() {
            // Building tls from tls_config already happens during builder
            // creation so this will never return an error.
            endpoint = endpoint.tls_config(tls_config).unwrap();
        }

        if let Some(tcp_nodelay) = self.tcp_nodelay {
            endpoint = endpoint.tcp_nodelay(tcp_nodelay);
        }

        if let Some(interval) = self.http2_keep_alive_interval {
            endpoint = endpoint.http2_keep_alive_interval(interval);
        }

        if let Some(duration) = self.http2_keep_alive_timeout {
            endpoint = endpoint.keep_alive_timeout(duration);
        }

        if let Some(enabled) = self.http2_keep_alive_while_idle {
            endpoint = endpoint.keep_alive_while_idle(enabled);
        }

        if let Some(enabled) = self.http2_adaptive_window {
            endpoint = endpoint.http2_adaptive_window(enabled);
        }

        endpoint
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    fn build_uri(&self, ip_addr: IpAddr) -> Uri {
        // We make sure this conversion doesn't return any errors in new already
        // so unwrap is safe here.
        let mut url = self.url.clone();
        url.set_ip_host(ip_addr.into()).unwrap();
        Uri::from_str(url.as_str()).unwrap()
    }
}
