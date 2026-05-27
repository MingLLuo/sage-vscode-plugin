"""Module loading helpers for the workspace index.

This module is a traceable split point for future extraction.  The current
implementation remains in :mod:`sage_lsp.workspace_index` to preserve behavior
while the cleanup lands incrementally.
"""

from .workspace_index import (  # noqa: F401
    default_index_cache_dir,
    file_fingerprint,
    merge_module_records,
    module_name_from_path,
    module_record_precedence,
    path_from_uri,
)
