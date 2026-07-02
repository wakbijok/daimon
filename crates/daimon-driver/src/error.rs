//! Driver-layer errors.
//!
//! A driver reaches infrastructure only through `daimon_broker::Broker::execute`
//! (D21) — so most failures surface as a `Broker` error wrapped here. `Param`
//! is the injection-chokepoint rejection (`param::validate`); `Unsupported` is
//! the "this driver does not implement that capability" case.

use thiserror::Error;

/// Errors a `Driver` verb can return.
#[derive(Debug, Error)]
pub enum DriverError {
    /// A supplied parameter failed its declared character-class allowlist
    /// (`param::validate`) — the single injection chokepoint. Rejected BEFORE
    /// any `Op` is built (FR-CON-12).
    #[error("parameter validation: {0}")]
    Param(#[from] crate::param::ParamError),

    /// The driver does not implement the requested capability/selector, or the
    /// target class does not match.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// The underlying brokered `ExecRequest` failed (inventory miss, vault
    /// resolution, transport, guard/policy denial, …). The credential boundary
    /// stays inside the broker — the driver only ever sees the stringified error.
    #[error("broker: {0}")]
    Broker(String),

    /// The transport returned an `OpResult`/status the driver could not parse
    /// into its typed return (`StateDoc`/`Finding`/`Receipt`/`TargetShape`).
    #[error("parse: {0}")]
    Parse(String),

    /// Catch-all for driver-internal failures.
    #[error("{0}")]
    Other(String),
}

impl From<daimon_broker::BrokerError> for DriverError {
    fn from(e: daimon_broker::BrokerError) -> Self {
        DriverError::Broker(e.to_string())
    }
}

/// Convenience alias for driver-verb results.
pub type DriverResult<T> = Result<T, DriverError>;
