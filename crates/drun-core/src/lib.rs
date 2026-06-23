//! drun-core: sandboxed execution engine. Owns Config, DrunEngine, Session,
//! and the checkpoint/snapshot model.

mod checkpoint;
pub mod config;
mod engine;
pub mod error;
mod proxy;
mod runner;
mod sandbox;
mod session;
mod snapshot;
mod workspace;

pub use checkpoint::{Checkpoint, CheckpointRef, FileMap};
pub use config::Config;
pub use engine::DrunEngine;
pub use error::RunnerError;
pub use session::Session;
pub use snapshot::{SessionSnapshot, SnapshotMeta};
