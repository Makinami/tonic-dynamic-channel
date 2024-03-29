mod endpoint_template;
pub use endpoint_template::EndpointTemplate;

mod dns;
use dns::resolve_domain;

use std::{collections::HashSet, net::IpAddr, time::Duration};

use tokio::{sync::watch::{self, Receiver}, task::JoinHandle};
use tonic::transport::Channel;
use tower::discover::Change;

pub struct AutoBalancedChannel {
    channel: Channel,
    background_task: JoinHandle<()>,
    status_reader: Receiver<Status>,
}

#[derive(Clone, Debug)]
pub enum Status {
    Ok,
    DnsResolutionError { details: String },
    Stopped,
}

impl Status {
    fn dns_resolution_error(e: impl std::fmt::Debug) -> Self {
        Self::DnsResolutionError { details: format!("{e:?}") }
    }
}

impl AutoBalancedChannel {
    const DEFAULT_INTERVAL: Duration = Duration::from_secs(15);

    pub fn new(endpoint_template: EndpointTemplate) -> Self {
        Self::with_interval(endpoint_template, Self::DEFAULT_INTERVAL)
    }

    pub fn with_interval(endpoint_template: EndpointTemplate, interval: Duration) -> AutoBalancedChannel {
        let (channel, sender) = Channel::balance_channel::<IpAddr>(1024);
        let (status_setter, status_reader) = watch::channel::<Status>(Status::Ok);

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
                let new_endpoints = match resolve_domain(domain) {
                    Ok(ip_addrs) => {
                        let _ = status_setter.send(Status::Ok);
                        ip_addrs.collect()
                    },
                    Err(e) => {
                        // DNS resolution errors might be recoverable and
                        // usually do not immediately spell doom for the
                        // channel. Because of this, we just report the interim
                        // problem and use last known IP addresses.
                        let _ = status_setter.send(Status::dns_resolution_error(e));
                        old_endpoints.clone()
                    },
                };

                for new_ip in new_endpoints.difference(&old_endpoints) {
                    if add_endpoint(*new_ip).await.is_err() {
                        // Receiver is closed which happens when crated Channel
                        // was dropped. There is nothing more to do, so we just
                        // report the status and quit.
                        let _ = status_setter.send(Status::Stopped);
                        return;
                    }
                }

                for old_ip in old_endpoints.difference(&new_endpoints) {
                    if sender.send(Change::Remove(*old_ip)).await.is_err() {
                        // Receiver is closed which happens when crated Channel
                        // was dropped. There is nothing more to do, so we just
                        // report the status and quit.
                        let _ = status_setter.send(Status::Stopped);
                        return;
                    }
                }

                old_endpoints = new_endpoints;

                interval.tick().await;
            }
        });

        Self {
            channel,
            background_task,
            status_reader,
        }
    }

    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    pub fn get_status(&self) -> Status {
        self.status_reader.borrow().to_owned()
    }
}

impl Drop for AutoBalancedChannel {
    fn drop(&mut self) {
        self.background_task.abort()
    }
}
