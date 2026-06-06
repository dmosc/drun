import { loadPyodide } from "npm:pyodide";

// Redirect all Pyodide console output to stderr to keep stdout clean for the
// JSON parsing.
// TODO(#1): Remove once generic implementation for output funneling is in place.
const textEncoder = new TextEncoder();
const toStderr = (...args: unknown[]) => Deno.stderr.writeSync(textEncoder.encode(args.join(" ") + "\n"));
console.log = toStderr;
console.info = toStderr;
console.warn = toStderr;

const [workspacePath, codePath] = Deno.args;
const code = await Deno.readTextFile(codePath);

const pyodide = await loadPyodide();
await pyodide.loadPackage("micropip", { messageCallback: toStderr, errorCallback: toStderr });

let stdout = "";
pyodide.setStdout({ batched: (text: string) => { stdout += text + "\n"; } });
pyodide.setStderr({ batched: (text: string) => { Deno.stderr.writeSync(textEncoder.encode(text + "\n")); } });

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
  Deno.stderr.writeSync(textEncoder.encode(String(err)));
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

Deno.stdout.writeSync(textEncoder.encode(JSON.stringify({ stdout: stdout.trimEnd(), files: collectFiles("/workspace") }) + "\n"));