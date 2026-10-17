//! Global allocator. musl's allocator (used by the static Docker build) is slow under
//! concurrency and holds on to freed memory, so mimalloc is used instead and tuned to
//! return memory to the OS promptly: a large scan shouldn't leave memory resident.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// Option numbers from the `mi_option_e` enum in the vendored mimalloc.h.
const ARENA_EAGER_COMMIT: libmimalloc_sys::mi_option_t = 4;
const PURGE_DELAY: libmimalloc_sys::mi_option_t = 15;

/// Call first thing in `main`. Equivalent to `MIMALLOC_ARENA_EAGER_COMMIT=0` and
/// `MIMALLOC_PURGE_DELAY=0`.
pub fn tune() {
    // SAFETY: setting mimalloc options has no memory-safety requirements.
    unsafe {
        libmimalloc_sys::mi_option_set(ARENA_EAGER_COMMIT, 0);
        libmimalloc_sys::mi_option_set(PURGE_DELAY, 0);
    }
}

/// Returns freed memory to the OS; worth calling after large, short-lived allocations.
pub fn release_free_memory() {
    // SAFETY: `mi_collect` may be called at any time from any thread.
    unsafe { libmimalloc_sys::mi_collect(true) }
}
