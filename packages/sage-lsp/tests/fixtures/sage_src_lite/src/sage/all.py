"""
Simplified fixture for sage.all.
"""

from sage.misc.lazy_import import lazy_import

from sage.env import SAGE_ROOT
from sage.functions.all import *
from sage.rings.all import *

lazy_import("sage.calculus.predefined", "x")
