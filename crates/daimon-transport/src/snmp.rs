//! SNMP v2c transport (READ-ONLY) via `csnmp` (P5-4).
//!
//! Device telemetry — interface counters, sysUpTime, sensor OIDs — from network
//! and storage gear that speaks SNMP but no REST/SSH API. Read-only by design:
//! `SnmpGet` (one OID) and `SnmpWalk` (a subtree) are supported; `SnmpSet` is
//! refused (SNMP writes are rare and high-risk — a later phase, behind approval).
//!
//! Auth: SNMP v2c uses a **community string**, resolved by reference from the
//! vault — `Credential::ApiToken{token}` (the community) or
//! `Credential::Generic{fields["community"]}`. It is never in the profile and,
//! like every credential, is borrowed per-request and never stored on the client.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use csnmp::{ObjectIdentifier, ObjectValue, Snmp2cClient};
use daimon_vault::Credential;
use tracing::instrument;

use crate::op::{Op, OpResult, SnmpValue, TransportError};
use crate::transport::{Transport, TransportTarget};

const TRANSPORT_ID: &str = "snmp";
const DEFAULT_PORT: u16 = 161;

pub struct SnmpTransport {
    timeout: Duration,
    retries: usize,
}

impl Default for SnmpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl SnmpTransport {
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            retries: 2,
        }
    }
}

#[async_trait]
impl Transport for SnmpTransport {
    fn id(&self) -> &str {
        TRANSPORT_ID
    }

    #[instrument(skip(self, cred, op), fields(host = %target.host, port = target.port))]
    async fn execute(
        &self,
        target: &TransportTarget,
        op: &Op,
        cred: &Credential,
    ) -> Result<OpResult, TransportError> {
        // Fail fast on unsupported ops BEFORE opening a socket.
        match op {
            Op::SnmpGet { .. } | Op::SnmpWalk { .. } => {}
            Op::SnmpSet { .. } => {
                return Err(TransportError::OpMismatch {
                    op: "snmp_set".into(),
                    transport: format!("{TRANSPORT_ID} is READ-ONLY in this build (SET disabled)"),
                });
            }
            _ => {
                return Err(TransportError::OpMismatch {
                    op: "non-snmp".into(),
                    transport: format!("{TRANSPORT_ID} requires an SNMP op (get/walk)"),
                });
            }
        }

        let community = community_from_cred(cred)?;
        let addr = resolve_addr(&target.host, target.port).await?;
        let client = Snmp2cClient::new(
            addr,
            community.into_bytes(),
            None,
            Some(self.timeout),
            self.retries,
        )
        .await
        .map_err(|e| TransportError::Connect(format!("snmp connect {addr}: {e}")))?;

        match op {
            Op::SnmpGet { oid } => {
                let o = parse_oid(oid)?;
                let v = client
                    .get(o)
                    .await
                    .map_err(|e| TransportError::Io(format!("snmp get {oid}: {e}")))?;
                let mut values = BTreeMap::new();
                values.insert(oid.clone(), map_value(&v));
                Ok(OpResult::Snmp { values })
            }
            Op::SnmpWalk { oid_root } => {
                let o = parse_oid(oid_root)?;
                let map = client
                    .walk(o)
                    .await
                    .map_err(|e| TransportError::Io(format!("snmp walk {oid_root}: {e}")))?;
                let values = map
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), map_value(&v)))
                    .collect();
                Ok(OpResult::Snmp { values })
            }
            _ => unreachable!("filtered above"),
        }
    }
}

/// The community string for SNMP v2c, resolved from the vault credential.
fn community_from_cred(cred: &Credential) -> Result<String, TransportError> {
    match cred {
        Credential::ApiToken { token } => Ok(token.clone()),
        Credential::Generic { fields } => fields
            .get("community")
            .cloned()
            .ok_or_else(|| TransportError::Auth("snmp Generic credential missing `community`".into())),
        other => Err(TransportError::OpMismatch {
            op: "snmp".into(),
            transport: format!(
                "{TRANSPORT_ID} requires ApiToken (community) or Generic{{community}}, got {:?}",
                other.kind()
            ),
        }),
    }
}

/// Resolve host:port to a `SocketAddr` (host may be an IP or a name).
async fn resolve_addr(host: &str, port: u16) -> Result<SocketAddr, TransportError> {
    let port = if port == 0 { DEFAULT_PORT } else { port };
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| TransportError::Connect(format!("resolve {host}:{port}: {e}")))?
        .next()
        .ok_or_else(|| TransportError::Connect(format!("no address for {host}:{port}")))
}

fn parse_oid(s: &str) -> Result<ObjectIdentifier, TransportError> {
    s.parse::<ObjectIdentifier>()
        .map_err(|e| TransportError::Other(format!("bad OID `{s}`: {e:?}")))
}

/// Map a csnmp `ObjectValue` into daimon's transport-neutral `SnmpValue`.
fn map_value(v: &ObjectValue) -> SnmpValue {
    match v {
        ObjectValue::Integer(i) => SnmpValue::Int(*i as i64),
        ObjectValue::Counter32(u) | ObjectValue::Unsigned32(u) | ObjectValue::TimeTicks(u) => {
            SnmpValue::Int(*u as i64)
        }
        ObjectValue::Counter64(u) => SnmpValue::Int(*u as i64),
        ObjectValue::String(bytes) => SnmpValue::String(String::from_utf8_lossy(bytes).into_owned()),
        ObjectValue::ObjectId(oid) => SnmpValue::Oid(oid.to_string()),
        ObjectValue::IpAddress(ip) => SnmpValue::String(ip.to_string()),
        ObjectValue::Opaque(bytes) => {
            SnmpValue::String(bytes.iter().map(|b| format!("{b:02x}")).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;

    #[test]
    fn map_value_covers_the_variants() {
        assert!(matches!(map_value(&ObjectValue::Integer(-5)), SnmpValue::Int(-5)));
        assert!(matches!(map_value(&ObjectValue::Counter32(42)), SnmpValue::Int(42)));
        assert!(matches!(map_value(&ObjectValue::TimeTicks(100)), SnmpValue::Int(100)));
        assert!(matches!(map_value(&ObjectValue::Counter64(9)), SnmpValue::Int(9)));
        match map_value(&ObjectValue::String(b"eth0".to_vec())) {
            SnmpValue::String(s) => assert_eq!(s, "eth0"),
            other => panic!("expected String, got {other:?}"),
        }
        match map_value(&ObjectValue::IpAddress("10.0.0.1".parse().unwrap())) {
            SnmpValue::String(s) => assert_eq!(s, "10.0.0.1"),
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn community_resolution_and_oid_parse() {
        let c = community_from_cred(&Credential::ApiToken {
            token: "public".into(),
        })
        .unwrap();
        assert_eq!(c, "public");

        let mut fields = Map::new();
        fields.insert("community".to_string(), "private".to_string());
        let c = community_from_cred(&Credential::Generic { fields }).unwrap();
        assert_eq!(c, "private");

        // A Generic without `community` is rejected.
        assert!(community_from_cred(&Credential::Generic { fields: Map::new() }).is_err());

        assert!(parse_oid("1.3.6.1.2.1.1.3.0").is_ok());
        assert!(parse_oid("not.an.oid").is_err());
    }

    #[tokio::test]
    async fn set_is_refused_read_only() {
        let t = SnmpTransport::new();
        let err = t
            .execute(
                &TransportTarget {
                    host: "127.0.0.1".into(),
                    port: 161,
                },
                &Op::SnmpSet {
                    oid: "1.3.6.1".into(),
                    value: SnmpValue::Int(1),
                },
                &Credential::ApiToken {
                    token: "public".into(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::OpMismatch { .. }));
    }
}
