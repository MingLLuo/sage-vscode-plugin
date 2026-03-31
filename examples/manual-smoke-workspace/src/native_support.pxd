"""Typed declarations used to smoke-test native Sage/Cython components."""

cdef class NativeAccumulator:
    cdef int value

cpdef int native_step(int value)
cdef inline int native_hidden_square(int value)
