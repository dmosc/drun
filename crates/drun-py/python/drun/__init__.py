import os

from .drun_internal import execute as _execute_internal
from .drun_internal import DrunOutput, DrunCheckpoint, DrunSession
from .utils import commit

__all__ = ["execute", "Session", "DrunOutput", "DrunCheckpoint"]


def execute(code: str, mounts=None, interactive=True) -> DrunOutput:
    result = _execute_internal(code, mounts)
    commit(result, interactive)
    return result


class Session:
    def __init__(self, files=None):
        self._inner = DrunSession(files)

    def execute(self, code: str) -> DrunCheckpoint:
        return self._inner.execute(code)

    def rollback(self, id: int) -> None:
        self._inner.rollback(id)

    def export(self, path: str, dest: str = None) -> None:
        files = self.current.files
        if path not in files:
            raise FileNotFoundError(f"'{path}' not in current checkpoint")
        dest_path = dest or path
        os.makedirs(os.path.dirname(os.path.abspath(dest_path)), exist_ok=True)
        with open(dest_path, 'wb') as f:
            f.write(bytes(files[path]))

    def export_all(self, dest_dir: str = '.') -> None:
        for path, content in self.current.files.items():
            dest_path = os.path.join(dest_dir, path)
            os.makedirs(os.path.dirname(
                os.path.abspath(dest_path)), exist_ok=True)
            with open(dest_path, 'wb') as f:
                f.write(bytes(content))

    @property
    def current(self) -> DrunCheckpoint:
        return self._inner.current

    @property
    def history(self) -> list:
        return self._inner.history
