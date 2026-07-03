//! Channel adapters. Each is behind its own Cargo feature so a deployment
//! compiles and exposes only the channels it uses (FR-GW-05).

#[cfg(feature = "telegram")]
pub mod telegram;

#[cfg(feature = "matrix")]
pub mod matrix;
