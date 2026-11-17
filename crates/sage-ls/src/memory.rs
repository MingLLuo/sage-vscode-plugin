//! Returning memory freed after index jobs to the operating system.

/// Returns memory freed by an index job to the operating system. Parsing a full Sage tree
/// briefly holds every file's symbols; without this the resident size stays at that peak.
pub(crate) fn release_freed_memory() {
    // Parsing runs on the rayon pool, so freed pages sit in those threads' heaps; collect on
    // every pool thread and then on the calling thread.
    // SAFETY: `mi_collect` only inspects and releases the allocator's own free pages.
    rayon::broadcast(|_| unsafe { libmimalloc_sys::mi_collect(true) });
    unsafe { libmimalloc_sys::mi_collect(true) };
    // The bundled SQLite allocates through the C library allocator, which keeps freed pages
    // resident after a bulk persist; ask it to return them.
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: a null zone means "all zones"; a zero goal releases as much as possible.
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: glibc's malloc_trim only releases free heap memory.
    unsafe {
        libc::malloc_trim(0);
    }
}
