//! `daimon-gateway` — messaging gateways (SRS §4.8 FR-GW-*; SDS §9).
//!
//! A gateway is a **transport, not a second brain**. An inbound message from a
//! chat platform (Telegram, Matrix, …) is verified, normalised, bound to a real
//! daimon IAM identity, and handed to the **same** Harness turn path a browser
//! message takes; the reply flows back out the same channel. The Harness, Guard,
//! broker, and audit spine are untouched — a gateway never becomes a privilege
//! side-channel (SDS §9.1 design posture).
//!
//! ## Module map
//! - [`reply_sink`] — the enabling refactor (FR-GW-01/02/03): `TurnEvent`,
//!   `ReplySink`, and the batched `BufferSink`/`OutboundChannel` pair. The chat
//!   loop emits `TurnEvent`s into a `&mut dyn ReplySink`; the browser socket is
//!   one impl (`WsSink`, in daimon-app), a channel adapter is another.
//! - `gateway` (commit 2) — the `Gateway` trait + `InboundMessage` + the
//!   webhook/poller ingress split + per-adapter signature verification.
//! - `adapters` (commits 4/5) — Telegram (webhook) and Matrix (/sync poller).
//!
//! ## D21 boundary
//! This crate depends on `daimon-core` (+ `daimon-broker` once secret resolution
//! is wired) and MUST NOT depend on `daimon-vault` / `daimon-inventory` /
//! `daimon-transport`. Credentials reach a gateway only by reference through the
//! broker. A CI grep gate on `Cargo.toml` enforces this.

pub mod adapters;
pub mod gateway;
pub mod reply_sink;
#[cfg(feature = "verify")]
pub mod verify;

pub use gateway::{
    AlertBody, ChannelId, Correlation, Gateway, GatewayError, InboundHandler, InboundHttp,
    InboundMessage, Ingress, PollingGateway, Recipient,
};
pub use reply_sink::{BufferSink, OutboundChannel, ReplySink, TurnEvent};
