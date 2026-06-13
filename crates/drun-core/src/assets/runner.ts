// @ts-nocheck
import { loadPyodide } from "npm:pyodide";

const enc = new TextEncoder();
const toStderr = (...args: unknown[]) =>
  Deno.stderr.writeSync(enc.encode(args.join(" ") + "\n"));
console.log = toStderr;
console.info = toStderr;
console.warn = toStderr;

const pyodide = await loadPyodide();
await pyodide.loadPackage("micropip", { messageCallback: toStderr, errorCallback: toStderr });

let capturedStdout = "";
let capturedStderr = "";
pyodide.setStdout({
  batched: (s: string) => {
    capturedStdout += s + "\n";
    Deno.stdout.writeSync(enc.encode(JSON.stringify({ progress: s }) + "\n"));
  }
});
pyodide.setStderr({ batched: (s: string) => { capturedStderr += s + "\n"; } });

function clearDir(path: string): void {
  for (const name of (pyodide.FS.readdir(path) as string[]).filter((n: string) => n !== "." && n !== "..")) {
    const child = `${path}/${name}`;
    const stat = pyodide.FS.stat(child);
    if (pyodide.FS.isDir(stat.mode)) {
      clearDir(child);
      pyodide.FS.rmdir(child);
    } else {
      pyodide.FS.unlink(child);
    }
  }
}

function syncWorkspace(files: Record<string, number[]>): void {
  try { clearDir("/workspace"); } catch { /* first run */ }
  try { pyodide.FS.mkdir("/workspace"); } catch { /* already exists */ }
  for (const [path, bytes] of Object.entries(files)) {
    let dir = "/workspace";
    for (const part of path.split("/").slice(0, -1)) {
      dir += `/${part}`;
      try { pyodide.FS.mkdir(dir); } catch { }
    }
    pyodide.FS.writeFile(`/workspace/${path}`, new Uint8Array(bytes));
  }
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

const dec = new TextDecoder();
let buf = "";
async function readLine(): Promise<string | null> {
  const chunk = new Uint8Array(4096);
  while (true) {
    const nl = buf.indexOf("\n");
    if (nl >= 0) {
      const line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      return line;
    }
    const n = await Deno.stdin.read(chunk);
    if (n === null) return null;
    buf += dec.decode(chunk.subarray(0, n));
  }
}

while (true) {
  const line = await readLine();
  if (line === null) break;
  if (!line.trim()) continue;

  const message = JSON.parse(line) as Record<string, unknown>;
  capturedStdout = "";
  capturedStderr = "";

  if ("package" in message) {
    try {
      pyodide.globals.set("_drun_pkg", String(message.package));
      await pyodide.runPythonAsync(`import micropip\nawait micropip.install(_drun_pkg)`);
      Deno.stdout.writeSync(enc.encode(JSON.stringify({ stdout: "", stderr: "", files: {} }) + "\n"));
    } catch (e) {
      Deno.stdout.writeSync(enc.encode(JSON.stringify({ error: String(e) }) + "\n"));
    } finally {
      pyodide.globals.delete("_drun_pkg");
    }
  } else {
    const { code, files } = message as { code: string; files: Record<string, number[]> };
    syncWorkspace(files);
    capturedStdout = "";
    capturedStderr = "";
    try {
      await pyodide.runPythonAsync(code);
      Deno.stdout.writeSync(enc.encode(JSON.stringify({
        stdout: capturedStdout.trimEnd(),
        stderr: capturedStderr.trimEnd(),
        files: collectFiles("/workspace"),
      }) + "\n"));
    } catch (e) {
      Deno.stdout.writeSync(enc.encode(JSON.stringify({ error: String(e) }) + "\n"));
    }
  }
}
