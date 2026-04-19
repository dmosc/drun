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
        let mut config = Config::new();
        config.consume_fuel(true);
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
        store.set_fuel(500_000_000)?;
        let instance = self.linker.instantiate(&mut store, &self.module)?;
        let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
        // Execute code in WASM runtime.
        if let Err(error) = start.call(&mut store, ()) {
            if let Some(trap) = error.downcast_ref::<wasmtime::Trap>() {
                if *trap == wasmtime::Trap::OutOfFuel {
                    anyhow::bail!("Ran out of fuel. Aborting execution.");
                }
            }
            anyhow::bail!("Runtime error: {}", error);
        }
        // Store outputs in tempfiles.
        let mut result = String::new();
        let mut file = stdout_temp;
        file.seek(SeekFrom::Start(0))?;
        file.read_to_string(&mut result)?;

        Ok(result)
    }
}
