//! Stateful execution session backed by a long-lived Deno subprocess. Manages
//! the checkpoint history and rollback.

use crate::{Checkpoint, DrunEngine, NetworkPolicy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout};

pub struct Session {
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    _child: Child,
    checkpoints: Vec<Checkpoint>,
}

#[derive(Serialize)]
struct ExecRequest<'a> {
    code: &'a str,
    files: &'a HashMap<String, Vec<u8>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExecResponse {
    Ok {
        stdout: String,
        files: HashMap<String, Vec<u8>>,
    },
    Err {
        error: String,
    },
}

impl Session {
    pub fn new(engine: &DrunEngine, network: NetworkPolicy) -> anyhow::Result<Self> {
        let mut child = engine.spawn_runner(network)?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            stdin,
            stdout,
            _child: child,
            checkpoints: vec![Checkpoint {
                id: 0,
                stdout: String::new(),
                files: HashMap::new(),
            }],
        })
    }

    pub fn mount(&mut self, files: HashMap<String, Vec<u8>>) {
        self.checkpoints.last_mut().unwrap().files.extend(files);
    }

    pub fn execute(&mut self, code: &str) -> anyhow::Result<&Checkpoint> {
        let files = &self.checkpoints.last().unwrap().files;
        let request = serde_json::to_string(&ExecRequest { code, files })?;
        writeln!(self.stdin, "{}", request)?;
        self.stdin.flush()?;

        let mut line = String::new();
        self.stdout.read_line(&mut line)?;

        match serde_json::from_str::<ExecResponse>(&line)? {
            ExecResponse::Ok { stdout, files } => {
                let id = self.checkpoints.len();
                self.checkpoints.push(Checkpoint { id, stdout, files });
                Ok(self.checkpoints.last().unwrap())
            }
            ExecResponse::Err { error } => anyhow::bail!(error),
        }
    }

    pub fn rollback(&mut self, id: usize) -> anyhow::Result<()> {
        if id >= self.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", id);
        }
        self.checkpoints.truncate(id + 1);
        Ok(())
    }

    pub fn diff(&self, from_id: usize, to_id: usize) -> anyhow::Result<String> {
        if from_id >= self.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", from_id);
        }
        if to_id >= self.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", to_id);
        }
        let from = &self.checkpoints[from_id].files;
        let to = &self.checkpoints[to_id].files;
        let keys: std::collections::BTreeSet<&String> = from.keys().chain(to.keys()).collect();
        let mut output = String::new();
        for key in keys {
            let from_bytes = from.get(key).map(Vec::as_slice).unwrap_or(&[]);
            let to_bytes = to.get(key).map(Vec::as_slice).unwrap_or(&[]);
            if from_bytes == to_bytes {
                continue;
            }
            match (
                std::str::from_utf8(from_bytes),
                std::str::from_utf8(to_bytes),
            ) {
                (Ok(a), Ok(b)) => output.push_str(
                    &similar::TextDiff::from_lines(a, b)
                        .unified_diff()
                        .header(&format!("a/{key}"), &format!("b/{key}"))
                        .to_string(),
                ),
                _ => output.push_str(&format!("Binary file {key} changed.\n")),
            }
        }
        Ok(output)
    }

    pub fn current(&self) -> &Checkpoint {
        self.checkpoints.last().unwrap()
    }

    pub fn history(&self) -> &[Checkpoint] {
        &self.checkpoints
    }
}
