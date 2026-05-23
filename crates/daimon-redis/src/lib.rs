//! Redis tier (#3 of the 5-DB storage architecture). Phase 4 D4.
//!
//! `WorkingMemory` is the trait every agent talks to for hot, short-TTL
//! state — chat session history, plan-in-flight handoffs, distributed
//! locks, kill-switch signal channel (defence-in-depth on top of the
//! filesystem flag from Phase 5).
//!
//! Two impls ship today:
//! - `RedisWorkingMemory` — production default. deadpool-redis pool.
//! - `InProcWorkingMemory` — tests + dev when Redis is unavailable. Maps to
//!   a tokio `RwLock<HashMap>` per category. NOT durable across process
//!   restarts.

pub mod error;
pub mod inproc;
pub mod redis_impl;
pub mod traits;

pub use error::{Error, Result};
pub use inproc::InProcWorkingMemory;
pub use redis_impl::RedisWorkingMemory;
pub use traits::{ConvMessage, WorkingMemory};
