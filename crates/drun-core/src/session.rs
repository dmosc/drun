use crate::DrunEngine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout};

pub struct Checkpoint {
    pub id: usize,
    pub stdout: String,
    pub files: HashMap<String, Vec<u8>>,
}

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
    pub fn new(engine: &DrunEngine) -> anyhow::Result<Self> {
        Self::with_files(engine, HashMap::new())
    }

    pub fn with_files(
        engine: &DrunEngine,
        files: HashMap<String, Vec<u8>>,
    ) -> anyhow::Result<Self> {
        let mut child = engine.spawn_runner()?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            stdin,
            stdout,
            _child: child,
            checkpoints: vec![Checkpoint {
                id: 0,
                stdout: String::new(),
                files,
            }],
        })
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

    pub fn current(&self) -> &Checkpoint {
        self.checkpoints.last().unwrap()
    }

    pub fn history(&self) -> &[Checkpoint] {
        &self.checkpoints
    }
}
