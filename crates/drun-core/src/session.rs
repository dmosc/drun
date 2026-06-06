use crate::DrunEngine;
use std::collections::HashMap;

pub struct Checkpoint {
    pub id: usize,
    pub stdout: String,
    pub files: HashMap<String, Vec<u8>>,
}

pub struct Session {
    engine: DrunEngine,
    checkpoints: Vec<Checkpoint>,
}

impl Session {
    pub fn new(engine: DrunEngine) -> Self {
        Self::with_files(engine, HashMap::new())
    }

    pub fn with_files(engine: DrunEngine, files: HashMap<String, Vec<u8>>) -> Self {
        Self {
            engine,
            checkpoints: vec![Checkpoint {
                id: 0,
                stdout: String::new(),
                files,
            }],
        }
    }

    pub fn execute(&mut self, code: &str) -> anyhow::Result<&Checkpoint> {
        let files = self.checkpoints.last().unwrap().files.clone();
        let output = self.engine.run_python_from_files(code, files)?;
        let id = self.checkpoints.len();
        self.checkpoints.push(Checkpoint {
            id,
            stdout: output.stdout,
            files: output.files,
        });
        Ok(self.checkpoints.last().unwrap())
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
