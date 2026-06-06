from .drun_internal import execute as _execute_internal
from .drun_internal import DrunOutput, DrunCheckpoint, DrunSession
from .utils import commit

__all__ = ["execute", "Session", "DrunOutput", "DrunCheckpoint"]


def execute(code: str, mounts=None, interactive=True) -> DrunOutput:
    result = _execute_internal(code, mounts)
    commit(result, interactive)
    return result


class Session:
    def __init__(self, files=None, interactive=True):
        self._inner = DrunSession(files)
        self.interactive = interactive

    def execute(self, code: str) -> DrunCheckpoint:
        checkpoint = self._inner.execute(code)
        commit(checkpoint, self.interactive)
        return checkpoint

    def rollback(self, id: int) -> None:
        self._inner.rollback(id)

    @property
    def current(self) -> DrunCheckpoint:
        return self._inner.current

    @property
    def history(self) -> list:
        return self._inner.history
