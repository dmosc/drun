use std::io::{Read, Seek, SeekFrom};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::{WasiCtxBuilder, cli::OutputFile, p1::WasiP1Ctx};

pub struct DrunEngine {
    engine: Engine,
    linker: Linker<WasiP1Ctx>,
    module: Module,
}

impl DrunEngine {
    pub fn new() -> anyhow::Result<Self> {
        let config = Config::new();
        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |t| t)?;
        let wasm_bytes = include_bytes!("assets/python-3.12.0.wasm");
        let module = Module::from_binary(&engine, wasm_bytes)?;

        Ok(Self {
            engine,
            linker,
            module,
        })
    }

    pub fn run_python(&self, code: &str) -> anyhow::Result<String> {
        // Initialize temp files to store runtime output.
        let stdout_temp = tempfile::tempfile()?;
        let stderr_temp = tempfile::tempfile()?;
        // Initialize a WASM runtime.
        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder
            .stdout(OutputFile::new(stdout_temp.try_clone()?))
            .stderr(OutputFile::new(stderr_temp.try_clone()?))
            .args(&["python", "-c", code]);
        let mut store = Store::new(&self.engine, wasi_builder.build_p1());
        let instance = self.linker.instantiate(&mut store, &self.module)?;
        let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
        // Execute code in WASM runtime.
        if let Err(e) = start.call(&mut store, ()) {
            if !e.to_string().contains("exit status 0") {
                anyhow::bail!("Python error: {}", e);
            }
        }
        // Store outputs in tempfiles.
        let mut result = String::new();
        let mut file = stdout_temp;
        file.seek(SeekFrom::Start(0))?;
        file.read_to_string(&mut result)?;

        Ok(result)
    }
}
