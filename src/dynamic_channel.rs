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
    dns_status_reader: Receiver<DnsStatus>,
    endpoints_count_reader: Receiver<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DnsStatus {
    Ok,
    ResolutionError { details: String },
}

impl DnsStatus {
    fn resolution_error(e: impl std::fmt::Debug) -> Self {
        Self::ResolutionError {
            details: format!("{e:?}"),
        }
    }

    fn is_error(&self) -> bool {
        match &self {
            Self::ResolutionError { .. } => true,
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
        let (dns_status_setter, dns_status_reader) = watch::channel::<DnsStatus>(DnsStatus::Ok);
        let (endpoints_count_setter, endpoints_count_reader) = watch::channel::<usize>(0);

        let background_task = tokio::spawn(async move {
            let add_endpoint = |ip_address: IpAddr| {
                let new_endpoint = endpoint_template.build(ip_address);
                sender.send(Change::Insert(ip_address, new_endpoint))
            };

            let mut old_endpoints: HashSet<IpAddr> = HashSet::new();
            let mut interval = tokio::time::interval(interval);
            loop {
                if sender.is_closed() {
                    return;
                }

                match resolve_domain(endpoint_template.domain()) {
                    Ok(ip_addrs) => {
                        let _ = dns_status_setter.send(DnsStatus::Ok);
                        let new_endpoints: HashSet<IpAddr> = ip_addrs.collect();

                        for new_ip in new_endpoints.difference(&old_endpoints) {
                            let _ = add_endpoint(*new_ip).await;
                        }

                        for old_ip in old_endpoints.difference(&new_endpoints) {
                            let _ = sender.send(Change::Remove(*old_ip)).await;
                        }

                        old_endpoints = new_endpoints;

                        let _ = endpoints_count_setter.send(old_endpoints.len());
                    }
                    Err(e) => {
                        // DNS resolution errors might be recoverable and does
                        // not necessarily spell doom for the channel. Because
                        // of this, we just report the interim problem and use
                        // last known IP addresses.
                        let _ = dns_status_setter.send(DnsStatus::resolution_error(e));
                    }
                };

                interval.tick().await;
            }
        });

        Self {
            channel,
            background_task,
            dns_status_reader,
            endpoints_count_reader,
        }
    }

    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }

    pub fn get_dns_status(&self) -> DnsStatus {
        self.dns_status_reader.borrow().to_owned()
    }

    pub fn get_health(&self) -> Health {
        if *self.endpoints_count_reader.borrow() == 0 {
            Health::Broken
        } else if self.dns_status_reader.borrow().is_error() {
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
