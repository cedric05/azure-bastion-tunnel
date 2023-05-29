use std::sync::Arc;

use azure_identity::{AutoRefreshingTokenCredential, AzureCliCredential};
use cfg_if::cfg_if;
use clap::Parser;

use tokio::signal;

use url::Url;
pub mod cli;
pub mod handler;
pub mod serve;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() -> std::result::Result<(), Error> {
    let params = cli::Cli::parse();

    // bind socket
    let listener = serve::Listener::bind(params.local).await?;
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

    let credential = AutoRefreshingTokenCredential::new(Arc::new(AzureCliCredential::default()));

    let mut handler = handler::AzTokenHandler {
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
                let connect = listener.accept().await;
                if let Ok(socket) = connect {
                    let auth_token = handler.get_next_url().await?;
                    let url = Url::parse(&auth_token)?;
                    println!("bastion url is {}", url);
                    tokio::spawn({
                        async {
                            socket.copy(url).await.unwrap_or(());
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
