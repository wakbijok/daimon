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

    /// Raw GET — returns JSON string for debugging/discovery
    pub async fn raw_get(&self, path: &str) -> Result<String, Error> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        Ok(resp.text().await?)
    }

    /// GET /api2/json/cluster/resources — all resources in one call
    pub async fn cluster_resources(&self, resource_type: Option<&str>) -> Result<Vec<PveResource>, Error> {
        let url = match resource_type {
            Some(t) => format!("{}/api2/json/cluster/resources?type={}", self.base_url, t),
            None => format!("{}/api2/json/cluster/resources", self.base_url),
        };
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<PveResource>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/status — detailed node info
    pub async fn node_status(&self, node: &str) -> Result<PveNodeStatus, Error> {
        let url = format!("{}/api2/json/nodes/{}/status", self.base_url, node);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<PveNodeStatus> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes
    pub async fn nodes(&self) -> Result<Vec<PveNode>, Error> {
        let url = format!("{}/api2/json/nodes", self.base_url);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        eprintln!("[daimon-pve] raw /nodes response: {}", &text[..text.len().min(500)]);
        let body: ApiResponse<Vec<PveNode>> = serde_json::from_str(&text)?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/qemu
    pub async fn node_qemu(&self, node: &str) -> Result<Vec<PveVm>, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu", self.base_url, node);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        eprintln!("[daimon-pve] raw /qemu response: {}", &text[..text.len().min(800)]);
        let body: ApiResponse<Vec<PveVm>> = serde_json::from_str(&text)?;
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

    /// GET /api2/json/nodes/{node}/rrddata — node historical metrics
    pub async fn node_rrddata(&self, node: &str, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/rrddata?timeframe={}", self.base_url, node, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/qemu/{vmid}/rrddata — VM historical metrics
    pub async fn qemu_rrddata(&self, node: &str, vmid: u32, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu/{}/rrddata?timeframe={}", self.base_url, node, vmid, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/lxc/{vmid}/rrddata — LXC historical metrics
    pub async fn lxc_rrddata(&self, node: &str, vmid: u32, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/lxc/{}/rrddata?timeframe={}", self.base_url, node, vmid, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/storage/{storage}/rrddata — storage historical metrics
    pub async fn storage_rrddata(&self, node: &str, storage: &str, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/storage/{}/rrddata?timeframe={}", self.base_url, node, storage, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/qemu/{vmid}/status/current — detailed VM status
    pub async fn qemu_status(&self, node: &str, vmid: u32) -> Result<crate::QemuStatus, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu/{}/status/current", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<crate::QemuStatus> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/lxc/{vmid}/status/current — detailed LXC status
    pub async fn lxc_status(&self, node: &str, vmid: u32) -> Result<crate::LxcStatus, Error> {
        let url = format!("{}/api2/json/nodes/{}/lxc/{}/status/current", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<crate::LxcStatus> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/qemu/{vmid}/config — VM configuration
    pub async fn qemu_config(&self, node: &str, vmid: u32) -> Result<crate::GuestConfig, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu/{}/config", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        let raw: ApiResponse<serde_json::Value> = serde_json::from_str(&text)?;
        let mut config: crate::GuestConfig = serde_json::from_value(raw.data.clone())?;
        if let Some(obj) = raw.data.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    if k.starts_with("net") && k[3..].parse::<u32>().is_ok() {
                        config.net_devices.push(s.to_string());
                    } else if (k.starts_with("scsi") && k[4..].parse::<u32>().is_ok())
                        || (k.starts_with("ide") && k[3..].parse::<u32>().is_ok())
                        || (k.starts_with("virtio") && k[6..].parse::<u32>().is_ok())
                        || (k.starts_with("sata") && k[4..].parse::<u32>().is_ok())
                    {
                        config.disk_devices.push(format!("{}: {}", k, s));
                    }
                }
            }
        }
        Ok(config)
    }

    /// GET /api2/json/nodes/{node}/lxc/{vmid}/config — LXC configuration
    pub async fn lxc_config(&self, node: &str, vmid: u32) -> Result<crate::GuestConfig, Error> {
        let url = format!("{}/api2/json/nodes/{}/lxc/{}/config", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        let raw: ApiResponse<serde_json::Value> = serde_json::from_str(&text)?;
        let mut config: crate::GuestConfig = serde_json::from_value(raw.data.clone())?;
        if let Some(obj) = raw.data.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    if k.starts_with("net") && k[3..].parse::<u32>().is_ok() {
                        config.net_devices.push(s.to_string());
                    } else if k.starts_with("rootfs") || (k.starts_with("mp") && k[2..].parse::<u32>().is_ok()) {
                        config.disk_devices.push(format!("{}: {}", k, s));
                    }
                }
            }
        }
        Ok(config)
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

    #[test]
    fn rrddata_url_uses_timeframe_string() {
        assert_eq!(crate::RrdTimeframe::Hour.as_str(), "hour");
        assert_eq!(crate::RrdTimeframe::Day.as_str(), "day");
        assert_eq!(crate::RrdTimeframe::Week.as_str(), "week");
        assert_eq!(crate::RrdTimeframe::Month.as_str(), "month");
        assert_eq!(crate::RrdTimeframe::Year.as_str(), "year");
    }
}
