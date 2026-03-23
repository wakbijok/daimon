use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Deserialize;

use crate::error::Error;
use crate::types::PveVersion;

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

    /// GET /api2/json/version — basic connectivity check
    pub async fn version(&self) -> Result<PveVersion, Error> {
        let url = format!("{}/api2/json/version", self.base_url);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<PveVersion> = resp.json().await?;
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
}
