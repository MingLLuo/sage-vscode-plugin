DEF TRACE_LIMIT = 32

cdef inline int included_native_step(int value):
    return value + 1
