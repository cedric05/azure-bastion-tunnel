use std::net::Ipv4Addr;
use std::sync::Arc;

use azure_identity::AzureCliCredential;
use cfg_if::cfg_if;
use clap::Parser;
use futures::{SinkExt, StreamExt};

use reqwest::header::HeaderMap;
use reqwest::Client;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::signal;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

/// Bastion auth token request's response data structure
#[derive(Deserialize, Serialize, Debug)]
struct BastionAuthTokenResponse {
    #[serde(alias = "authToken")]
    auth_token: String,
    #[serde(alias = "name")]
    username: String,
    #[serde(alias = "dataSource")]
    data_source: String,
    #[serde(alias = "nodeId")]
    node_id: String,
    #[serde(alias = "availableDataSources")]
    available_data_sources: Vec<String>,
}

/// Handler for getting new auth tokens
struct Handler {
    /// currently it uses AzCliTokenCredentials only
    credential: Box<dyn azure_core::auth::TokenCredential>,
    client: Client,
    last_token: Option<String>,
    node_id: Option<String>,
    resource_id: String,
    bastion: String,
    tunnel: String,
    port: u16,
}

mod cli;

impl Handler {
    /// Requests new auth token from bastion
    async fn get_next_url(
        &mut self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync + 'static>> {
        // first get az access token
        let az_token = self
            .credential
            .get_token("https://management.azure.com")
            .await?
            .token
            .secret()
            .to_owned();
        let last_token = if let Some(last_token) = self.last_token.as_ref() {
            last_token.clone()
        } else {
            "".to_string()
        };
        let content = [
            ("resourceId", self.resource_id.clone()),
            ("protocol", self.tunnel.clone()),
            ("workloadHostPort", self.port.to_string()),
            ("aztoken", az_token),
            ("token", last_token),
        ];
        let web_address = format!("https://{}/api/tokens", self.bastion);
        let mut hadermap: HeaderMap = Default::default();
        if let Some(node_id) = self.node_id.as_ref() {
            hadermap.append("X-Node-Id", node_id.try_into()?);
        }
        // get auth_token from bastion
        let response: BastionAuthTokenResponse = self
            .client
            .post(web_address)
            .form(&content)
            .headers(hadermap)
            .send()
            .await?
            .json()
            .await?;

        // generate url
        let host = format!(
            "wss://{}/webtunnel/{}?X-Node-Id={}",
            self.bastion, response.auth_token, response.node_id
        );
        self.node_id = Some(response.node_id);
        self.last_token = Some(response.auth_token);
        Ok(host)
    }

    /// deletes generated auth_token/session from bastion
    async fn delete(self) -> Result<(), Error> {
        if let Some(last_token) = self.last_token {
            let web_address = format!("https://{}/api/tokens/{}", self.bastion, last_token);
            let status = self.client.delete(web_address).send().await?.status();
            if status == StatusCode::NOT_FOUND {
                println!("nothing to delete");
            }
        }
        Ok(())
    }
}

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() -> std::result::Result<(), Error> {
    let params = cli::Cli::parse();

    // get subscription from terminal or use default subscription
    let subscription_id = params
        .subscription_id
        .unwrap_or(AzureCliCredential::get_subscription()?);
    // get bastion address
    let bastion = {
        let client =
            azure_mgmt_network::Client::builder(Arc::new(AzureCliCredential::default())).build();
        let bastion = client
            .bastion_hosts_client()
            .get(
                params.resource_group.clone(),
                params.bastion,
                subscription_id.clone(),
            )
            .into_future()
            .await?;

        bastion
            .properties
            .expect("expect properties to be present on bastion")
            .dns_name
            .expect("expect dns name attached to bastion")
    };

    let credential = AzureCliCredential::default();
    let mut handler = Handler {
        resource_id: format!(
            "/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Compute/virtualMachines/{}",
            subscription_id, params.resource_group, params.vm
        ),
        bastion,
        tunnel: params.tunnel,
        port: params.remote_port,
        last_token: None,
        node_id: None,
        client: Default::default(),
        credential: Box::new(credential),
    };

    // bind socket
    let tcp = TcpListener::bind((Ipv4Addr::from([127, 0, 0, 1]), params.local_port)).await?;

    cfg_if! {
        if #[cfg(unix)] {
            let mut term_sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let fut = tokio::spawn(async move {
                tokio::select! {
                    _ = term_sig.recv()=>{
                    },
                    _ = signal::ctrl_c() => {
                    }
                }
            });
        } else {
            let fut = tokio::spawn(async move {
                tokio::select! {
                    _ = signal::ctrl_c() => {
                    }
                }
            });
        }
    };

    tokio::select!(
        _ = async {
            // listen for new connections
            println!("listening for new connections");
            loop {
                let connect = tcp.accept().await;
                if let Ok((socket, _addr)) = connect {
                    println!("new connection with addr {:?}", _addr);
                    let auth_token = handler.get_next_url().await?;
                    let url = Url::parse(&auth_token)?;
                    println!("bastion url is {}", url);
                    tokio::spawn({
                        async {
                            copy(socket, url).await.unwrap_or(());
                        }
                    });
                } else {
                    break;
                };
            }
            Ok::<(), Error>(())
        }=>{},
        // delete created session auth tokens
        _ = fut=>{
            println!("closing connections and deleting sessions");
            handler.delete().await?;
        }
    );
    Ok(())
}

// proxy local connection to websocket
async fn copy(socket: TcpStream, url: Url) -> std::result::Result<(), Error> {
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    let (mut writer, mut reader) = (ws_stream).split();
    let (mut socket_reader, mut socket_writer) = socket.into_split();
    let mut buf = [0; 4096];
    tokio::select! {
        r = async {
            loop{
                match socket_reader.read(&mut buf).await? {
                    0 => {
                        break;
                    }
                    n => {
                        writer.send(Message::binary(&buf[..n])).await?;
                    }
                };
            }
            Ok(())
        }=>r,
        r = async {
            loop{
                let message = reader.next().await;
                if let Some(data) = message {
                    let binary_data = data?;
                    let data = binary_data.into_data();
                    socket_writer.write_all(&data).await?;
                } else {
                    break;
                }
            }
            Ok(())
        } => r
    }
}
