//! Real REST/HTTP transport via `reqwest` (D7, FR-CON-15).
//!
//! Three of the four reference target classes (Kubernetes / vCenter / cloud APIs)
//! speak pure REST and are dead until this transport exists. It follows the
//! exact pattern `SshTransport` set: `execute` matches on `Op::Http`, returns
//! `TransportError::OpMismatch` for non-HTTP ops (mirroring `ssh.rs`), reads the
//! credential BY REFERENCE without cloning the secret into long-lived client
//! state, and returns `OpResult::Http`.
//!
//! # TLS
//!
//! Built on `rustls` (`reqwest` with `default-features = false`,
//! `features = ["rustls-tls","json"]`). Certificate validation is ON by default
//! and there is deliberately NO `danger_accept_invalid_certs` path — the
//! removed PVE-client crate's SSRF/`danger_accept_invalid_certs` surface is
//! exactly what this transport must not re-introduce.
//!
//! # Scheme
//!
//! `Op::Http.path` is a path (or absolute URL). When it is a bare path the
//! transport composes `https://{host}:{port}{path}`. HTTPS is the default so
//! the validate-by-default posture actually applies; a caller that genuinely
//! needs plaintext must pass a full `http://…` URL in `path` (rare, and
//! visible in audit).
//!
//! # Credentials
//!
//! The resolved `&Credential` is borrowed for exactly the duration of the call.
//! `Credential::ApiToken` is injected as an `Authorization: Bearer <token>`
//! header (built per-request from the borrow, never stored on the client). The
//! `Credential` type is `ZeroizeOnDrop`; the broker owns it and wipes it when
//! `execute` returns. Non-token credential kinds return `OpMismatch`.

use std::time::Duration;

use async_trait::async_trait;
use daimon_vault::Credential;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Client;
use tracing::{debug, instrument};

use crate::op::{HttpMethod, Op, OpResult, TransportError};
use crate::transport::{Transport, TransportTarget};

const TRANSPORT_ID: &str = "rest";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// REST transport. Construct via [`RestTransport::new`] (rustls, validate-by-
/// default, 30s per-op timeout) or [`RestTransport::with_timeout`].
///
/// The `reqwest::Client` holds a connection pool but NO credential state —
/// auth is applied per-request from the borrowed `&Credential`.
#[derive(Clone)]
pub struct RestTransport {
    client: Client,
    timeout: Duration,
}

impl Default for RestTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl RestTransport {
    /// Production default: rustls, certificate validation ON, 30s per-op
    /// timeout. Panics only if the platform TLS backend cannot be initialized
    /// (unrecoverable — same posture as `reqwest::Client::new`).
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
    }

    /// As [`RestTransport::new`] but with a custom per-op timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        // rustls, validate-by-default. NO danger_accept_invalid_certs — ever.
        let client = Client::builder()
            .use_rustls_tls()
            .build()
            .expect("build rustls reqwest client");
        Self { client, timeout }
    }

    fn full_url(target: &TransportTarget, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_owned()
        } else {
            let sep = if path.starts_with('/') { "" } else { "/" };
            format!("https://{}:{}{}{}", target.host, target.port, sep, path)
        }
    }

    #[instrument(skip(self, cred, headers, body), fields(host = %target.host, port = target.port, method = ?method))]
    async fn exec_http(
        &self,
        target: &TransportTarget,
        method: &HttpMethod,
        path: &str,
        headers: &std::collections::BTreeMap<String, String>,
        body: &Option<serde_json::Value>,
        cred: &Credential,
    ) -> Result<OpResult, TransportError> {
        let url = Self::full_url(target, path);

        let mut builder = self
            .client
            .request(to_reqwest_method(method), &url)
            .timeout(self.timeout);

        // Static headers from the op.
        for (k, v) in headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        // Inject auth from the borrowed credential — built per-request, never
        // stored on the client. Non-token kinds are a transport/credential
        // mismatch (mirrors ssh.rs's kind check).
        match cred {
            Credential::ApiToken { token } => {
                let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|e| TransportError::Auth(format!("invalid bearer token: {e}")))?;
                auth.set_sensitive(true);
                builder = builder.header(AUTHORIZATION, auth);
            }
            other => {
                return Err(TransportError::OpMismatch {
                    op: "http".into(),
                    transport: format!(
                        "{TRANSPORT_ID} requires ApiToken credential, got {:?}",
                        other.kind()
                    ),
                });
            }
        }

        if let Some(b) = body {
            builder = builder.json(b);
        }

        debug!(url = %url, "rest: dispatching request");

        let resp = builder.send().await.map_err(map_reqwest_err)?;
        let status = resp.status().as_u16();
        let resp_headers = collect_headers(resp.headers());

        // Read the body as text, then parse to JSON where possible so
        // OpResult::Http.body stays a serde_json::Value. Non-JSON bodies are
        // wrapped as a JSON string so nothing is lost.
        let text = resp
            .text()
            .await
            .map_err(|e| TransportError::Io(format!("read body: {e}")))?;
        let json_body = if text.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str::<serde_json::Value>(&text)
                .unwrap_or_else(|_| serde_json::Value::String(text))
        };

        Ok(OpResult::Http {
            status,
            body: json_body,
            headers: resp_headers,
        })
    }
}

#[async_trait]
impl Transport for RestTransport {
    fn id(&self) -> &str {
        TRANSPORT_ID
    }

    async fn execute(
        &self,
        target: &TransportTarget,
        op: &Op,
        cred: &Credential,
    ) -> Result<OpResult, TransportError> {
        match op {
            Op::Http {
                method,
                path,
                headers,
                body,
            } => self.exec_http(target, method, path, headers, body, cred).await,
            other => Err(TransportError::OpMismatch {
                op: op_name(other).into(),
                transport: TRANSPORT_ID.into(),
            }),
        }
    }
}

fn to_reqwest_method(m: &HttpMethod) -> reqwest::Method {
    match m {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
    }
}

fn collect_headers(headers: &HeaderMap) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            out.insert(name.as_str().to_owned(), v.to_owned());
        }
    }
    out
}

fn map_reqwest_err(e: reqwest::Error) -> TransportError {
    if e.is_timeout() {
        // We set a per-op timeout above; surface it as Timeout.
        TransportError::Timeout(DEFAULT_TIMEOUT_SECS as u32)
    } else if e.is_connect() {
        TransportError::Connect(format!("rest connect: {e}"))
    } else {
        TransportError::Io(format!("rest: {e}"))
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::ShellCommand { .. } => "shell_command",
        Op::Http { .. } => "http",
        Op::SnmpGet { .. } => "snmp_get",
        Op::SnmpSet { .. } => "snmp_set",
        Op::SnmpWalk { .. } => "snmp_walk",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::HttpMethod;
    use std::collections::BTreeMap;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn token_cred() -> Credential {
        Credential::ApiToken {
            token: "s3cr3t-token".into(),
        }
    }

    fn ssh_cred() -> Credential {
        Credential::SshPassword {
            username: "u".into(),
            password: "p".into(),
        }
    }

    fn target_for(server: &MockServer) -> (TransportTarget, String) {
        // MockServer::uri() is like "http://127.0.0.1:PORT". Split into host/port.
        let uri = server.uri();
        let stripped = uri.strip_prefix("http://").unwrap();
        let (host, port) = stripped.split_once(':').unwrap();
        (
            TransportTarget {
                host: host.to_owned(),
                port: port.parse().unwrap(),
            },
            uri,
        )
    }

    #[tokio::test]
    async fn id_is_rest() {
        assert_eq!(RestTransport::new().id(), "rest");
    }

    #[tokio::test]
    async fn get_returns_body_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api2/json/version"))
            .and(header("authorization", "Bearer s3cr3t-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": {"version": "8.1"}})),
            )
            .mount(&server)
            .await;

        let (mut target, base) = target_for(&server);
        // The mock server is plaintext http; pass a full http:// URL so the
        // transport does not force https (production targets are https).
        let _ = &mut target;
        let full = format!("{base}/api2/json/version");

        let transport = RestTransport::new();
        let result = transport
            .execute(
                &target,
                &Op::Http {
                    method: HttpMethod::Get,
                    path: full,
                    headers: BTreeMap::new(),
                    body: None,
                },
                &token_cred(),
            )
            .await
            .expect("GET should succeed");

        match result {
            OpResult::Http { status, body, .. } => {
                assert_eq!(status, 200);
                assert_eq!(body["data"]["version"], "8.1");
            }
            other => panic!("expected Http result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_http_op_returns_mismatch() {
        let transport = RestTransport::new();
        let err = transport
            .execute(
                &TransportTarget {
                    host: "127.0.0.1".into(),
                    port: 443,
                },
                &Op::ShellCommand {
                    command: "true".into(),
                    timeout_secs: 2,
                },
                &token_cred(),
            )
            .await
            .unwrap_err();
        match err {
            TransportError::OpMismatch { op, transport } => {
                assert_eq!(op, "shell_command");
                assert_eq!(transport, "rest");
            }
            other => panic!("expected OpMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_token_credential_returns_mismatch() {
        let server = MockServer::start().await;
        let (target, base) = target_for(&server);
        let full = format!("{base}/anything");
        let transport = RestTransport::new();
        let err = transport
            .execute(
                &target,
                &Op::Http {
                    method: HttpMethod::Get,
                    path: full,
                    headers: BTreeMap::new(),
                    body: None,
                },
                &ssh_cred(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::OpMismatch { .. }));
    }

    #[tokio::test]
    async fn bare_path_composes_https_url() {
        let target = TransportTarget {
            host: "api.example".into(),
            port: 8006,
        };
        assert_eq!(
            RestTransport::full_url(&target, "/api2/json/version"),
            "https://api.example:8006/api2/json/version"
        );
        assert_eq!(
            RestTransport::full_url(&target, "api2/json/version"),
            "https://api.example:8006/api2/json/version"
        );
        assert_eq!(
            RestTransport::full_url(&target, "http://other.host/x"),
            "http://other.host/x"
        );
    }
}
