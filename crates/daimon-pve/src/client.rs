use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Deserialize;

use crate::error::Error;
use crate::types::*;

/// Authentication method for the PVE API.
pub enum Auth {
    /// API token: PVEAPIToken=user@realm!tokenname=token-value
    Token {
        user: String,
        token_name: String,
        token_value: String,
    },
}

/// Proxmox VE API client.
pub struct Client {
    base_url: String,
    http: reqwest::Client,
}

/// Standard PVE API response wrapper: `{ "data": T }`
#[derive(Deserialize)]
struct ApiResponse<T> {
    data: T,
}

impl Client {
    pub fn new(base_url: &str, auth: Auth) -> Self {
        let mut headers = HeaderMap::new();

        match &auth {
            Auth::Token { user, token_name, token_value } => {
                let value = format!("PVEAPIToken={user}!{token_name}={token_value}");
                headers.insert(AUTHORIZATION, HeaderValue::from_str(&value).expect("valid token header"));
            }
        }

        // PVE uses self-signed certs by default
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build()
            .expect("failed to build HTTP client");

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    /// Build a Client from the stored token string format `user@realm!tokenname=value`.
    pub fn from_token_string(base_url: &str, token: &str) -> Self {
        let (user_part, token_value) = token.split_once('=').unwrap_or((token, ""));
        let (user, token_name) = user_part.split_once('!').unwrap_or((user_part, ""));
        Self::new(base_url, Auth::Token {
            user: user.to_string(),
            token_name: token_name.to_string(),
            token_value: token_value.to_string(),
        })
    }

    /// GET /api2/json/version — basic connectivity check
    pub async fn version(&self) -> Result<PveVersion, Error> {
        let url = format!("{}/api2/json/version", self.base_url);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<PveVersion> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes
    pub async fn nodes(&self) -> Result<Vec<PveNode>, Error> {
        let url = format!("{}/api2/json/nodes", self.base_url);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<PveNode>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/qemu
    pub async fn node_qemu(&self, node: &str) -> Result<Vec<PveVm>, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu", self.base_url, node);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<PveVm>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/lxc
    pub async fn node_lxc(&self, node: &str) -> Result<Vec<PveLxc>, Error> {
        let url = format!("{}/api2/json/nodes/{}/lxc", self.base_url, node);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<PveLxc>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/storage
    pub async fn storage(&self) -> Result<Vec<PveStorage>, Error> {
        let url = format!("{}/api2/json/storage", self.base_url);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<PveStorage>> = resp.json().await?;
        Ok(body.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_with_token() {
        let client = Client::new("https://localhost:8006", Auth::Token {
            user: "root@pam".into(),
            token_name: "test".into(),
            token_value: "00000000-0000-0000-0000-000000000000".into(),
        });
        assert!(client.base_url == "https://localhost:8006");
    }

    #[test]
    fn from_token_string_splits_on_bang() {
        let client = Client::from_token_string(
            "https://pve:8006",
            "root@pam!mytoken=aaaa-bbbb",
        );
        assert_eq!(client.base_url, "https://pve:8006");
    }

    #[test]
    fn from_token_string_no_bang_fallback() {
        let client = Client::from_token_string(
            "https://pve:8006",
            "root@pam",
        );
        assert_eq!(client.base_url, "https://pve:8006");
    }
}
