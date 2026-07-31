//! Push-based drunmon: counts tool calls per tool name and periodically
//! reports the running totals to an operator-configured collector. Disabled
//! unless drunmon_url is set in config.toml.

use crate::env::Env;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub(crate) struct ToolCallCounters {
    counts: Arc<Mutex<HashMap<String, u64>>>,
}

impl ToolCallCounters {
    pub(crate) fn increment(&self, tool_name: &str) {
        *self
            .counts
            .lock()
            .unwrap()
            .entry(tool_name.to_string())
            .or_insert(0) += 1;
    }

    pub(crate) fn snapshot(&self) -> HashMap<String, u64> {
        self.counts.lock().unwrap().clone()
    }
}

#[derive(Serialize)]
struct DrunmonPayload {
    instance_id: String,
    drun_version: &'static str,
    tool_calls: HashMap<String, u64>,
}

pub(crate) struct DrunmonReporter {
    instance_id: String,
    client: reqwest::Client,
}

impl DrunmonReporter {
    /// Loads this installation's persisted instance_id, generating and
    /// storing a new one under ~/.drun/instance_id on first run.
    pub(crate) fn load_or_create() -> Self {
        let path = Env.drun_home().join("instance_id");
        let instance_id = std::fs::read_to_string(&path)
            .ok()
            .map(|contents| contents.trim().to_string())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| {
                let id = uuid::Uuid::new_v4().to_string();
                if std::fs::create_dir_all(Env.drun_home()).is_ok() {
                    let _ = std::fs::write(&path, &id);
                }
                id
            });
        Self {
            instance_id,
            client: reqwest::Client::new(),
        }
    }

    pub(crate) async fn is_reachable(&self, endpoint: &str) -> bool {
        self.client
            .get(endpoint)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }

    pub(crate) async fn push(&self, endpoint: &str, tool_calls: HashMap<String, u64>) {
        let payload = DrunmonPayload {
            instance_id: self.instance_id.clone(),
            drun_version: env!("CARGO_PKG_VERSION"),
            tool_calls,
        };
        if let Err(e) = self.client.post(endpoint).json(&payload).send().await {
            eprintln!("drun: drunmon push to {endpoint} failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_accumulates_per_tool_name() {
        let counters = ToolCallCounters::default();
        counters.increment("create_session");
        counters.increment("create_session");
        counters.increment("session_bash");
        let snapshot = counters.snapshot();
        assert_eq!(snapshot["create_session"], 2);
        assert_eq!(snapshot["session_bash"], 1);
    }
}
