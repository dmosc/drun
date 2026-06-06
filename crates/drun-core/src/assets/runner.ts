import { loadPyodide } from "npm:pyodide";

const [workspacePath, codePath] = Deno.args;
const code = await Deno.readTextFile(codePath);

const pyodide = await loadPyodide();

let stdout = "";
pyodide.setStdout({ batched: (text: string) => { stdout += text + "\n"; } });
pyodide.setStderr({ batched: (text: string) => { Deno.stderr.writeSync(new TextEncoder().encode(text + "\n")); } });

async function mountDir(hostDir: string, pyDir: string): Promise<void> {
  try { pyodide.FS.mkdir(pyDir); } catch { /* already exists */ }
  for await (const entry of Deno.readDir(hostDir)) {
    const hostPath = `${hostDir}/${entry.name}`;
    const pyPath = `${pyDir}/${entry.name}`;
    if (entry.isDirectory) {
      await mountDir(hostPath, pyPath);
    } else {
      pyodide.FS.writeFile(pyPath, await Deno.readFile(hostPath));
    }
  }
}

await mountDir(workspacePath, "/workspace");

try {
  await pyodide.runPythonAsync(code);
} catch (err) {
  console.error(`${err}`);
  Deno.exit(1);
}

function collectFiles(dir: string): Record<string, number[]> {
  const out: Record<string, number[]> = {};
  for (const name of (pyodide.FS.readdir(dir) as string[]).filter((n: string) => n !== "." && n !== "..")) {
    const full = `${dir}/${name}`;
    const stat = pyodide.FS.stat(full);
    if (pyodide.FS.isFile(stat.mode)) {
      out[full.slice("/workspace/".length)] = Array.from(pyodide.FS.readFile(full) as Uint8Array);
    } else if (pyodide.FS.isDir(stat.mode)) {
      Object.assign(out, collectFiles(full));
    }
  }
  return out;
}

console.log(JSON.stringify({ stdout: stdout.trimEnd(), files: collectFiles("/workspace") }));
