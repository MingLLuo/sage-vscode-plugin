"""Simplified fixture for sage.functions.all."""

from sage.misc.lazy_import import lazy_import

lazy_import("sage.functions.other", ["sqrt", "factorial"])
