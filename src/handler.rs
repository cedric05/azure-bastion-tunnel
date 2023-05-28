use crate::Error;

use reqwest::header::HeaderMap;
use reqwest::Client;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::Serialize;

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
pub struct AzTokenHandler {
    /// currently it uses AzCliTokenCredentials only
    pub(crate) credential: Box<dyn azure_core::auth::TokenCredential>,
    pub(crate) client: Client,
    pub(crate) last_token: Option<String>,
    pub(crate) node_id: Option<String>,
    pub(crate) resource_id: String,
    pub(crate) bastion: String,
    pub(crate) tunnel: String,
    pub(crate) port: u16,
}

impl AzTokenHandler {
    /// Requests new auth token from bastion
    pub(crate) async fn get_next_url(
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
    pub(crate) async fn delete(self) -> Result<(), Error> {
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
