mod endpoint_template;
pub use endpoint_template::{EndpointTemplate, Error as EndpointTemplateError};

mod dns;
#[cfg(feature = "mock-dns")]
pub use dns::mock_net;

mod dynamic_channel;
pub use dynamic_channel::{AutoBalancedChannel, DnsStatus, Health};
