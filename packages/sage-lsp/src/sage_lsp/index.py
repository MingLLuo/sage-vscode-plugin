"""Compatibility facade for the Sage workspace index."""

from __future__ import annotations

import sys

from . import workspace_index as _workspace_index

sys.modules[__name__] = _workspace_index
