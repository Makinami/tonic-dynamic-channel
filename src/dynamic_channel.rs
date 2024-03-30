use crate::endpoint_template::EndpointTemplate;

use crate::dns::resolve_domain;

use std::{collections::HashSet, net::IpAddr, time::Duration};

use tokio::{
    sync::watch::{self, Receiver},
    task::JoinHandle,
};
use tonic::transport::Channel;
use tower::discover::Change;

pub struct AutoBalancedChannel {
    channel: Channel,
    background_task: JoinHandle<()>,
    status_reader: Receiver<Status>,
    endpoints_count_reader: Receiver<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Ok,
    DnsResolutionError { details: String },
}

impl Status {
    fn dns_resolution_error(e: impl std::fmt::Debug) -> Self {
        Self::DnsResolutionError {
            details: format!("{e:?}"),
        }
    }

    fn is_dns_resolution_error(&self) -> bool {
        match &self {
            Status::DnsResolutionError { .. } => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Health {
    /// There is at least one successfully detected and available endpoint
    Ok,
    /// Latest DNS resolution has failed, but there are still previously
    /// registered endpoints, so making gRPC calls could succeed.
    Undetermined,
    /// There are no endpoints available. Calling gRPC method will block until
    /// one is detected.
    Broken,
}

impl AutoBalancedChannel {
    const DEFAULT_INTERVAL: Duration = Duration::from_secs(15);

    pub fn new(endpoint_template: EndpointTemplate) -> Self {
        Self::with_interval(endpoint_template, Self::DEFAULT_INTERVAL)
    }

    pub fn with_interval(
        endpoint_template: EndpointTemplate,
        interval: Duration,
    ) -> AutoBalancedChannel {
        let (channel, sender) = Channel::balance_channel::<IpAddr>(16);
        let (status_setter, status_reader) = watch::channel::<Status>(Status::Ok);
        let (endpoints_count_setter, endpoints_count_reader) = watch::channel::<usize>(0);

        let background_task = tokio::spawn(async move {
            let add_endpoint = |ip_address: IpAddr| {
                let new_endpoint = endpoint_template.build(ip_address);
                sender.send(Change::Insert(ip_address, new_endpoint))
            };

            // We make sure that the URL contains a host when creating a
            // builder.
            let domain = match endpoint_template.url().host().unwrap() {
                url::Host::Domain(domain) => domain,
                // If provided URL already points to an IP address, there is
                // nothing to resolve. On top of that, there will never be more
                // than one address, so we can add it and return early from the
                // background task.
                url::Host::Ipv4(ip) => {
                    let _ = add_endpoint(ip.into()).await;
                    return;
                }
                url::Host::Ipv6(ip) => {
                    let _ = add_endpoint(ip.into()).await;
                    return;
                }
            };

            let mut old_endpoints: HashSet<IpAddr> = HashSet::new();
            let mut interval = tokio::time::interval(interval);
            loop {
                if sender.is_closed() {
                    return;
                }

                match resolve_domain(domain) {
                    Ok(ip_addrs) => {
                        let _ = status_setter.send(Status::Ok);
                        let new_endpoints: HashSet<IpAddr> = ip_addrs.collect();

                        for new_ip in new_endpoints.difference(&old_endpoints) {
                            let _ = add_endpoint(*new_ip).await;
                        }

                        for old_ip in old_endpoints.difference(&new_endpoints) {
                            let _ =  sender.send(Change::Remove(*old_ip)).await;
                        }

                        old_endpoints = new_endpoints;

                        let _ = endpoints_count_setter.send(old_endpoints.len());
                    }
                    Err(e) => {
                        // DNS resolution errors might be recoverable and
                        // usually do not immediately spell doom for the
                        // channel. Because of this, we just report the interim
                        // problem and use last known IP addresses.
                        let _ = status_setter.send(Status::dns_resolution_error(e));
                    }
                };

                interval.tick().await;
            }
        });

        Self {
            channel,
            background_task,
            status_reader,
            endpoints_count_reader,
        }
    }

    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }

    pub fn get_status(&self) -> Status {
        self.status_reader.borrow().to_owned()
    }

    pub fn get_health(&self) -> Health {
        if *self.endpoints_count_reader.borrow() == 0 {
            Health::Broken
        } else if self.status_reader.borrow().is_dns_resolution_error() {
            Health::Undetermined
        } else {
            Health::Ok
        }
    }
}

impl Drop for AutoBalancedChannel {
    fn drop(&mut self) {
        self.background_task.abort()
    }
}
