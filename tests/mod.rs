use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use sequential_test::sequential;
use tonic_dynamic_channel::{AutoBalancedChannel, EndpointTemplate, Status};
use tokio::task::JoinSet;
use tonic::{transport::Server, Request, Response};

use foo::foo_client::FooClient;
use foo::foo_server::{Foo, FooServer};
use foo::{Empty, ServerResponse};
use url::Url;

pub mod foo {
    tonic::include_proto!("foo");
}

#[derive(Debug, Default)]
pub struct MyServer {
    address: String,
}

impl MyServer {
    async fn run(address: impl Into<String>) -> Result<(), tonic::transport::Error> {
        let address = address.into();
        let server = Self {
            address: address.clone(),
        };
        Server::builder()
            .add_service(FooServer::new(server))
            .serve((address + ":50051").parse().unwrap())
            .await
    }
}

#[tonic::async_trait]
impl Foo for MyServer {
    async fn get_server(
        &self,
        _request: Request<Empty>, // Accept request of type HelloRequest
    ) -> Result<Response<ServerResponse>, tonic::Status> {
        // Return an instance of type HelloReply
        let reply = foo::ServerResponse {
            message: self.address.to_owned(), // We must use .into_inner() as the fields of gRPC requests and responses are private
        };

        Ok(Response::new(reply)) // Send back our formatted greeting
    }
}

fn set_dns(addresses: &[&str]) {
    let sockets = addresses
        .iter()
        .map(|address| std::net::IpAddr::from_str(address).unwrap())
        .map(|ip| std::net::SocketAddr::new(ip, 0))
        .collect::<Vec<_>>();
    tonic_dynamic_channel::mock_net::set_socket_addrs(Box::new(move |_, _| Ok(sockets.clone())));
}

fn setup() -> (JoinSet<Result<(), tonic::transport::Error>>, std::sync::Arc<AutoBalancedChannel>, std::sync::Arc<std::sync::RwLock<HashMap<String, i32>>>) {
    let mut set = JoinSet::new();

    set.spawn(async { MyServer::run("[::1]").await });
    set.spawn(async { MyServer::run("127.0.0.1").await });

    let balanced = Arc::new(AutoBalancedChannel::with_interval(
        EndpointTemplate::new(Url::parse("http://localhost:50051").expect("url fialed"))
            .expect("endpoint template"),
        Duration::from_millis(1),
    ));

    let responses: Arc<RwLock<HashMap<String, i32>>> = Arc::new(RwLock::new(HashMap::new()));

    {
        let balanced = balanced.clone();
        let client = FooClient::new(balanced.channel());
        let responses = responses.clone();
        set.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(10));
            loop {
                if balanced.get_status() == Status::Ok {
                    let response = client
                        .clone()
                        .get_server(tonic::Request::new(Empty {}))
                        .await
                        .expect("response");
                    let server = response.into_inner().message;
                    *responses
                        .write()
                        .expect("failed to get a write lock")
                        .entry(server)
                        .or_default() += 1;
                }

                interval.tick().await;
            }
        });
    }

    (set, balanced, responses)
}

#[tokio::test]
#[sequential]
async fn test_no_endpoints() {
    let (_set, balanced, _responses) = setup();

    set_dns(&[]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(balanced.get_status(), Status::NoEndpoints);
}

#[tokio::test]
#[sequential]
async fn test_balancing() {
    let (_set, _balanced, responses) = setup();

    set_dns(&["127.0.0.1", "::1"]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    responses.write().expect("can't get a write lock").clear();
    tokio::time::sleep(Duration::from_secs(1)).await;
    responses.write().and_then(|responses| {
        assert!(
            responses
                .get("127.0.0.1")
                .expect("no response from 127.0.0.1 server")
                >= &40,
            "strangely few responses from 127.0.0.1 server"
        );
        assert!(
            responses
                .get("[::1]")
                .expect("no response from [::1] server")
                >= &40,
            "strangely few responses from [::1] server"
        );
        Ok(())
    }).expect("can't get a write lock");
}

#[tokio::test]
#[sequential]
async fn test_switching() {
    let (_set, _balanced, responses) = setup();

    println!("only IPv4");
    set_dns(&["127.0.0.1"]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    responses.write().expect("can't get a write lock").clear();
    tokio::time::sleep(Duration::from_secs(1)).await;
    responses.read().and_then(|responses| {
        assert!(
            responses
                .get("127.0.0.1")
                .expect("no response from 127.0.0.1 server")
                >= &90,
            "strangely few responses from 127.0.0.1 server"
        );
        assert!(
            responses.get("[::1]").is_none(),
            "a response from [::1] was received"
        );
        Ok(())
    }).expect("can't get a read lock");

    println!("only IPv6");
    set_dns(&["::1"]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    responses.write().expect("can't get a write lock").clear();
    tokio::time::sleep(Duration::from_secs(1)).await;
    responses.read().and_then(|responses| {
        assert!(
            responses.get("127.0.0.1").is_none(),
            "a response from 127.0.0.1 was received"
        );
        assert!(
            responses
                .get("[::1]")
                .expect("no response from [::1] server")
                >= &90,
            "strangely few responses from [::1] server"
        );
        Ok(())
    }).expect("can't get a write lock");
}

#[tokio::test]
#[sequential]
async fn test_dns_error() {
    let (_set, balanced, _responses) = setup();

    set_dns(&["127.0.0.1", "::1"]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    tonic_dynamic_channel::mock_net::set_socket_addrs(Box::new(move |_, _| {
        #[derive(Debug)]
        struct Error {}
        impl std::fmt::Display for Error {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "Error")
            }
        }
        impl std::error::Error for Error {}
        Err(std::io::Error::new(std::io::ErrorKind::Other, Error {}))
    }));
    tokio::time::sleep(Duration::from_millis(10)).await;
    match balanced.get_status() {
        Status::DnsResolutionError { .. } => assert!(true),
        _ => assert!(false, "status is not DnsResolutionError"),
    }
}
