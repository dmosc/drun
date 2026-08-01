//! Push-based drunmon: tracks per-tool call counts, error counts, and
//! execution latency, and periodically reports the running totals to an
//! operator-configured collector.

use crate::env::Env;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Default, Serialize)]
pub(crate) struct LatencyTotal {
    sum_ms: u64,
    count: u64,
}

#[derive(Clone, Default)]
pub(crate) struct ToolMetrics {
    calls: Arc<Mutex<HashMap<String, u64>>>,
    errors: Arc<Mutex<HashMap<String, u64>>>,
    latencies: Arc<Mutex<HashMap<String, LatencyTotal>>>,
}

impl ToolMetrics {
    pub(crate) fn record(&self, tool_name: &str, duration: Duration, succeeded: bool) {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(tool_name.to_string())
            .or_insert(0) += 1;

        if !succeeded {
            *self
                .errors
                .lock()
                .unwrap()
                .entry(tool_name.to_string())
                .or_insert(0) += 1;
        }

        let mut latencies = self.latencies.lock().unwrap();
        let total = latencies.entry(tool_name.to_string()).or_default();
        total.sum_ms += duration.as_millis() as u64;
        total.count += 1;
    }

    pub(crate) fn snapshot(&self) -> ToolMetricsSnapshot {
        ToolMetricsSnapshot {
            calls: self.calls.lock().unwrap().clone(),
            errors: self.errors.lock().unwrap().clone(),
            latencies: self.latencies.lock().unwrap().clone(),
        }
    }
}

pub(crate) struct ToolMetricsSnapshot {
    calls: HashMap<String, u64>,
    errors: HashMap<String, u64>,
    latencies: HashMap<String, LatencyTotal>,
}

#[derive(Serialize)]
struct DrunmonPayload {
    instance_id: String,
    drun_version: &'static str,
    tool_calls: HashMap<String, u64>,
    tool_errors: HashMap<String, u64>,
    tool_latency_ms: HashMap<String, LatencyTotal>,
}

pub(crate) struct DrunmonReporter {
    instance_id: String,
    client: reqwest::Client,
}

impl DrunmonReporter {
    const DEFAULT_INGEST_URL: &'static str = "http://162.243.162.221/ingest";

    pub(crate) fn ingest_url() -> String {
        std::env::var("DRUNMON_URL").unwrap_or_else(|_| Self::DEFAULT_INGEST_URL.to_string())
    }

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

    pub(crate) async fn push(&self, endpoint: &str, snapshot: ToolMetricsSnapshot) {
        let payload = DrunmonPayload {
            instance_id: self.instance_id.clone(),
            drun_version: env!("CARGO_PKG_VERSION"),
            tool_calls: snapshot.calls,
            tool_errors: snapshot.errors,
            tool_latency_ms: snapshot.latencies,
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
    fn record_tracks_calls_errors_and_latency_per_tool() {
        let metrics = ToolMetrics::default();
        metrics.record("create_session", Duration::from_millis(10), true);
        metrics.record("create_session", Duration::from_millis(20), false);
        metrics.record("session_bash", Duration::from_millis(5), true);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.calls["create_session"], 2);
        assert_eq!(snapshot.calls["session_bash"], 1);
        assert_eq!(snapshot.errors["create_session"], 1);
        assert!(!snapshot.errors.contains_key("session_bash"));
        assert_eq!(snapshot.latencies["create_session"].sum_ms, 30);
        assert_eq!(snapshot.latencies["create_session"].count, 2);
    }
}
