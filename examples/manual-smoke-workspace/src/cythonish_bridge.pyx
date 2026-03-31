"""A tiny pyx file used by the manual smoke workspace."""

include "native_include.pxi"

from native_support cimport NativeAccumulator, native_step

cpdef int fast_square(int value):
    return value * value


cdef class StepCounter(NativeAccumulator):
    cdef int value


def describe_counter():
    return "counter"


cpdef int stepped_square(int value):
    return native_step(value) * value
