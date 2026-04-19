use std::io::{Read, Seek, SeekFrom};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder, cli::OutputFile, p1::WasiP1Ctx};

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
        // Setup host resources
        let stdout = tempfile::tempfile()?;
        let stderr = tempfile::tempfile()?;
        let workspace = tempfile::tempdir()?;
        // Initialize WASI context.
        let wasi_ctx = self.prepare_wasi_ctx(code, &workspace, &stdout, &stderr)?;
        // Initialize store and instance.
        let mut store = Store::new(&self.engine, wasi_ctx);
        store.set_fuel(500_000_000)?;
        let instance = self.linker.instantiate(&mut store, &self.module)?;
        let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
        // Execute WASM interpreter.
        self.execute_wasm(&mut store, start)?;
        // Cleanup and retrieve results.
        self.finalize_execution(workspace, stdout)
    }

    fn prepare_wasi_ctx(
        &self,
        code: &str,
        workspace: &tempfile::TempDir,
        stdout: &std::fs::File,
        stderr: &std::fs::File,
    ) -> anyhow::Result<wasmtime_wasi::p1::WasiP1Ctx> {
        let mut builder = WasiCtxBuilder::new();
        builder
            .preopened_dir(
                workspace.path(),
                "workspace",
                DirPerms::all(),
                FilePerms::all(),
            )?
            .stdout(OutputFile::new(stdout.try_clone()?))
            .stderr(OutputFile::new(stderr.try_clone()?))
            .args(&["python", "-c", code]);

        Ok(builder.build_p1())
    }

    fn execute_wasm(
        &self,
        store: &mut Store<wasmtime_wasi::p1::WasiP1Ctx>,
        func: wasmtime::TypedFunc<(), ()>,
    ) -> anyhow::Result<()> {
        if let Err(error) = func.call(store, ()) {
            if let Some(trap) = error.downcast_ref::<wasmtime::Trap>() {
                if *trap == wasmtime::Trap::OutOfFuel {
                    anyhow::bail!("Ran out of fuel. Aborting execution.");
                }
            }
            anyhow::bail!("Runtime error: {}", error);
        }

        Ok(())
    }

    fn finalize_execution(
        &self,
        workspace: tempfile::TempDir,
        mut stdout: std::fs::File,
    ) -> anyhow::Result<String> {
        for entry in std::fs::read_dir(workspace.path())? {
            let entry = entry?;
            println!("File created/modified: {:?}", entry.file_name());
        }

        let mut result = String::new();
        stdout.seek(SeekFrom::Start(0))?;
        stdout.read_to_string(&mut result)?;

        Ok(result)
    }
}
