from .drun_internal import execute as _execute_internal
from .drun_internal import DrunOutput
from .utils import commit

__all__ = ["execute", "DrunOutput"]


def execute(code: str, mounts=None, interactive=True) -> DrunOutput:
    result = _execute_internal(code, mounts)
    commit(result, interactive)
    return result
