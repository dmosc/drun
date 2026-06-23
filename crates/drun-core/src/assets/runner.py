import sys
import os
import atexit
import json
import shutil
import tempfile
import traceback
import subprocess

PACKAGES_DIR = sys.argv[1]
WORKSPACE_DIR = tempfile.mkdtemp(prefix='drun_ws_')
CAPTURE_LIMIT = 1024 * 1024

atexit.register(lambda: shutil.rmtree(WORKSPACE_DIR, ignore_errors=True))
sys.path.insert(0, PACKAGES_DIR)

protocol_out = sys.stdout
persistent_globals = {'__name__': '__main__'}


class ProgressWriter:
    def __init__(self):
        self.captured = ''

    def write(self, text):
        if text:
            protocol_out.write(json.dumps({'progress': text}) + '\n')
            protocol_out.flush()
            if len(self.captured) < CAPTURE_LIMIT:
                self.captured += text
        return len(text)

    def flush(self):
        pass


def sync_workspace(files):
    if os.path.exists(WORKSPACE_DIR):
        shutil.rmtree(WORKSPACE_DIR)
    os.makedirs(WORKSPACE_DIR)
    for path, byte_list in files.items():
        dest = os.path.join(WORKSPACE_DIR, path)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, 'wb') as f:
            f.write(bytes(byte_list))


def collect_workspace():
    result = {}
    for root, _, file_names in os.walk(WORKSPACE_DIR):
        for name in file_names:
            full = os.path.join(root, name)
            rel = os.path.relpath(full, WORKSPACE_DIR)
            with open(full, 'rb') as f:
                result[rel] = list(f.read())
    return result


def send(msg):
    protocol_out.write(json.dumps(msg) + '\n')
    protocol_out.flush()


send({'ready': True})

for raw_line in sys.stdin:
    line = raw_line.strip()
    if not line:
        continue

    msg = json.loads(line)

    if 'package' in msg:
        proxy_keys = {'http_proxy', 'https_proxy'}
        pip_env = {k: v for k, v in os.environ.items() if k not in proxy_keys}
        result = subprocess.run(
            [sys.executable, '-m', 'pip', 'install',
                '--target', PACKAGES_DIR, msg['package']],
            capture_output=True,
            text=True,
            env=pip_env,
        )
        if result.returncode == 0:
            send({'stdout': '', 'stderr': '', 'files': {}})
        else:
            send({'error': (result.stderr or result.stdout).strip()})
    else:
        sync_workspace(msg.get('files', {}))
        os.chdir(WORKSPACE_DIR)
        stdout_writer, stderr_writer = ProgressWriter(), ProgressWriter()
        prev_stdout, prev_stderr = sys.stdout, sys.stderr
        sys.stdout, sys.stderr = stdout_writer, stderr_writer
        try:
            exec(compile(msg['code'], '<drun>', 'exec'), persistent_globals)
            sys.stdout, sys.stderr = prev_stdout, prev_stderr
            send({
                'stdout': stdout_writer.captured.rstrip('\n'),
                'stderr': stderr_writer.captured.rstrip('\n'),
                'files': collect_workspace(),
            })
        except Exception:
            sys.stdout, sys.stderr = prev_stdout, prev_stderr
            send({'error': traceback.format_exc()})
