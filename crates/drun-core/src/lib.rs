use fs_extra::dir::CopyOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder, cli::OutputFile, p1::WasiP1Ctx};

pub struct DrunEngine {
    engine: Engine,
    linker: Linker<WasiP1Ctx>,
    module: Module,
}

pub struct DrunOutput {
    pub stdout: String,
    pub files: std::collections::HashMap<String, Vec<u8>>,
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

    pub fn run_python(&self, code: &str, mounts: Vec<String>) -> anyhow::Result<DrunOutput> {
        // Setup host resources
        let stdout = tempfile::tempfile()?;
        let stderr = tempfile::tempfile()?;
        let workspace = tempfile::tempdir()?;
        self.mount_assets(&workspace, mounts)?;
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
    ) -> anyhow::Result<DrunOutput> {
        let mut files = std::collections::HashMap::new();
        let base_path = workspace.path();
        for entry in walkdir::WalkDir::new(base_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let relative_path = path.strip_prefix(base_path)?.to_string_lossy().into_owned();
            let content = std::fs::read(path)?;
            files.insert(relative_path, content);
        }

        let mut stdout_str = String::new();
        stdout.seek(SeekFrom::Start(0))?;
        stdout.read_to_string(&mut stdout_str)?;

        Ok(DrunOutput {
            stdout: stdout_str,
            files,
        })
    }

    fn mount_assets(
        &self,
        workspace: &tempfile::TempDir,
        mounts: Vec<String>,
    ) -> anyhow::Result<()> {
        let target_base = workspace.path();
        for source in mounts {
            let source_path = Path::new(&source);
            if !source_path.exists() {
                anyhow::bail!("Mount source does not exist: {}", source);
            }

            let destination = target_base.join(&source);
            if source_path.is_dir() {
                std::fs::create_dir_all(&destination)?;
                let options = CopyOptions::new().overwrite(true).content_only(true);
                fs_extra::dir::copy(source_path, &destination, &options)?;
            } else {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(source_path, &destination)?;
            }
        }

        Ok(())
    }
}
