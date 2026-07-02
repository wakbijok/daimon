//! The `Driver` trait spine — daimon's platform-agnostic target-connector seam
//! (SDS §5.2, SRS FR-DRV-01..02, FR-CON-*).
//!
//! # The seam
//!
//! `TransportKind` (`Ssh | Rest | Snmp | Grpc`) says *how* to reach a target;
//! [`TargetClass`] says *what* the target is, so the LLM/orchestrator can pick
//! verbs. The two axes are orthogonal: a `Firewall` may be `Ssh` (RouterOS) or
//! `Rest` (FortiGate); `Compute` may be `Rest` (Proxmox, vCenter).
//!
//! Every driver implements [`Driver`]'s four class-uniform verbs
//! (`describe`/`read_state`/`diagnose`/`remediate`). The verbs give the
//! orchestrator and LLM a stable, vendor-independent vocabulary — "diagnose
//! this firewall" resolves identically whether the target is RouterOS-over-SSH
//! or FortiGate-over-REST.
//!
//! # The credential boundary is enforced by construction (D21)
//!
//! This crate depends ONLY on `daimon-core` + `daimon-broker`. It does NOT — and
//! by CI grep gate MUST NOT — depend on `daimon-vault`, `daimon-inventory`, or
//! `daimon-transport`. A driver therefore *structurally cannot* resolve a
//! credential, read the vault, or open a transport: the only way it reaches
//! infrastructure is by constructing an `ExecRequest` (with the resolved
//! `Capability` attached as `capability_meta`) and submitting it to
//! `daimon_broker::Broker::execute`. Guard → inventory → vault → transport →
//! zeroize → single audit event are all inherited unchanged. `Op` / `OpResult`
//! / `TargetRef` / `HttpMethod` are imported from `daimon_broker` (which
//! re-exports them, D21-legal), never from `daimon_transport`.
//!
//! Every verb ultimately dispatches through `Broker::execute` with the
//! capability's server-side read-only/irreversible/compensating disposition
//! attached — a driver NEVER touches vault or transport directly, and NEVER
//! supplies the read-only flag itself (that is derived server-side from the
//! registered `Capability`, closing the H6/H7 bypass).

pub mod error;
pub mod param;
pub mod types;

pub use error::{DriverError, DriverResult};
pub use param::{validate, ParamClass, ParamError};
pub use types::{Finding, Receipt, Severity, StateDoc, TargetShape};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The semantic peer to `TransportKind` — *what* a target is, orthogonal to
/// *how* it is reached (FR-CON-04). Lives here (not in inventory) because it is
/// a driver-facing concept; inventory must stay credential/transport-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetClass {
    /// Hypervisors / VM hosts: PVE, vCenter.
    Compute,
    /// Routers, switches, APs (L2/L3 device config).
    Network,
    /// NAS / SAN / object appliances.
    Storage,
    /// K8s API servers, Nomad, Swarm.
    Orchestrator,
    /// Packet-filter / policy devices: RouterOS filter, FortiGate, Palo.
    Firewall,
}

/// Derive the [`TargetClass`] from a capability name's leading namespace
/// segment (FR-CAP-02: `<class>.<vendor|subsystem>.<object>.<verb>`).
///
/// This is the ONLY place the string prefix is turned into the enum. Mapping:
///
/// - `network.*` → [`TargetClass::Firewall`] if the second segment names a
///   firewall vendor/subsystem (`firewall` / `routeros` / `fortigate` /
///   `palo`), otherwise [`TargetClass::Network`]. (RouterOS filter rules are
///   the firewall surface of a router, so the second segment decides.)
/// - `compute.*` → [`TargetClass::Compute`]
/// - `storage.*` → [`TargetClass::Storage`]
/// - `orchestrator.*` → [`TargetClass::Orchestrator`]
/// - anything else → [`TargetClass::Compute`] (documented conservative default).
pub fn parse_class(capability_name: &str) -> TargetClass {
    let mut segments = capability_name.split('.');
    let head = segments.next().unwrap_or("");
    match head {
        "network" => {
            // Second segment disambiguates a firewall surface from plain L2/L3.
            let second = segments.next().unwrap_or("");
            const FIREWALL_SUBSYSTEMS: &[&str] =
                &["firewall", "routeros", "fortigate", "palo", "paloalto"];
            if FIREWALL_SUBSYSTEMS.contains(&second) {
                TargetClass::Firewall
            } else {
                TargetClass::Network
            }
        }
        "compute" => TargetClass::Compute,
        "storage" => TargetClass::Storage,
        "orchestrator" => TargetClass::Orchestrator,
        // Conservative default: an unknown namespace is treated as Compute
        // rather than guessing a more privileged/specialized class.
        _ => TargetClass::Compute,
    }
}

/// The platform-agnostic connector contract (SDS §5.2.2, FR-DRV-01..02).
///
/// Four class-uniform verbs. Every write ultimately dispatches through
/// `daimon_broker::Broker::execute` with the capability's server-side
/// disposition attached — the driver never touches vault/transport directly.
///
/// - `describe` / `read_state` / `diagnose` are inherently READ (no approval).
/// - `remediate` is a WRITE, subject to full guard policy + approval; the write
///   disposition is derived server-side from the registered `Capability`
///   (FR-CON-05/13), never from the verb.
#[async_trait]
pub trait Driver: Send + Sync {
    /// Which class this driver serves — used by the harness to resolve a driver
    /// for a target/anomaly (FR-CON-19).
    fn class(&self) -> TargetClass;

    /// The versioned capabilities this driver implements. Registered into the
    /// `CapabilityRegistry` at boot (FR-CAP-05). SERVER-SIDE truth for the
    /// read-only / irreversible / compensating disposition (FR-CON-13).
    fn capabilities(&self) -> &[daimon_core::Capability];

    /// READ — the target's shape/topology as this driver understands it.
    /// Inherently read-only; no approval (FR-CON-05/06).
    async fn describe(&self, target: &str) -> DriverResult<TargetShape>;

    /// READ — a live typed snapshot for `selector` (e.g. `{"vm": 101}`,
    /// `{"table": "firewall.filter"}`). Inherently read-only (FR-CON-05/06/19).
    async fn read_state(&self, target: &str, selector: serde_json::Value)
        -> DriverResult<StateDoc>;

    /// READ — findings for a symptom; each carries a suggested remediation
    /// capability + params. Read-only. The LLM does root-cause over the output
    /// (FR-CON-06/20).
    async fn diagnose(&self, target: &str, symptom: &str) -> DriverResult<Vec<Finding>>;

    /// WRITE — apply `capability` with `params`; returns a [`Receipt`] that saga
    /// rollback derives compensating params from (FR-CON-06/22). Subject to full
    /// guard policy + approval; the write disposition is derived server-side
    /// from the registered `Capability`, never from this verb.
    async fn remediate(
        &self,
        target: &str,
        capability: &str,
        params: serde_json::Value,
    ) -> DriverResult<Receipt>;
}

#[cfg(test)]
mod parse_class_tests {
    use super::*;

    #[test]
    fn network_firewall_subsystems_map_to_firewall() {
        assert_eq!(
            parse_class("network.routeros.firewall_add_drop_rule"),
            TargetClass::Firewall
        );
        assert_eq!(
            parse_class("network.firewall.address_list.add"),
            TargetClass::Firewall
        );
        assert_eq!(
            parse_class("network.fortigate.policy.create"),
            TargetClass::Firewall
        );
    }

    #[test]
    fn network_non_firewall_maps_to_network() {
        assert_eq!(
            parse_class("network.switch.vlan.create"),
            TargetClass::Network
        );
        assert_eq!(parse_class("network.cisco.interface.list"), TargetClass::Network);
    }

    #[test]
    fn compute_storage_orchestrator_map_straight() {
        assert_eq!(parse_class("compute.pve.vm.start"), TargetClass::Compute);
        assert_eq!(parse_class("storage.ceph.pool.list"), TargetClass::Storage);
        assert_eq!(
            parse_class("orchestrator.k8s.deployment.patch_limits"),
            TargetClass::Orchestrator
        );
    }

    #[test]
    fn unknown_namespace_defaults_to_compute() {
        assert_eq!(parse_class("mystery.thing.do"), TargetClass::Compute);
        assert_eq!(parse_class(""), TargetClass::Compute);
    }

    #[test]
    fn target_class_serde_is_snake_case() {
        let json = serde_json::to_string(&TargetClass::Orchestrator).unwrap();
        assert_eq!(json, "\"orchestrator\"");
        let back: TargetClass = serde_json::from_str("\"firewall\"").unwrap();
        assert_eq!(back, TargetClass::Firewall);
    }
}
