"""A tiny pyx file used by the manual smoke workspace."""

cpdef int fast_square(int value):
    return value * value


cdef class StepCounter:
    cdef int value


def describe_counter():
    return "counter"
