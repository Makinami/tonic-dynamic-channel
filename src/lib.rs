mod endpoint_builder;
pub use endpoint_builder::EndpointBuilder;

mod dns;
use dns::resolve_domain;

use std::{collections::HashSet, net::IpAddr};

use tokio::task::JoinHandle;
use tonic::transport::Channel;
use tower::discover::Change;

pub struct AutoBalancedChannel {
    channel: Channel,
    background_task: JoinHandle<()>,
}

impl AutoBalancedChannel {
    pub fn from_endpoint(endpoint_builder: EndpointBuilder) -> AutoBalancedChannel {
        let (channel, sender) = Channel::balance_channel::<IpAddr>(1);

        let background_task = tokio::spawn(async move {
            let add_endpoint = |ip_address: IpAddr| {
                let new_endpoint = endpoint_builder.build(ip_address);
                // todo: what to do with errors?
                sender.send(Change::Insert(ip_address, new_endpoint))
            };

            // We make sure that the URL contains a host when creating a
            // builder.
            let domain = match endpoint_builder.url().host().unwrap() {
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
            loop {
                let new_endpoints = resolve_domain(domain)
                    .expect("dns resolution failed")
                    .collect::<HashSet<IpAddr>>();

                for new_ip in new_endpoints.difference(&old_endpoints) {
                    let _ = add_endpoint(*new_ip).await;
                }

                for old_ip in old_endpoints.difference(&new_endpoints) {
                    let _ = sender.send(Change::Remove(*old_ip)).await;
                }

                old_endpoints = new_endpoints;
            }
        });

        Self {
            channel,
            background_task,
        }
    }

    pub fn channel(&self) -> &Channel {
        &self.channel
    }
}

impl Drop for AutoBalancedChannel {
    fn drop(&mut self) {
        self.background_task.abort()
    }
}
