# PR 19 profiler safety correction

The original process-global queue assumed one producer, but independent plugin instances can process on different host threads. Two producers could read the same head and concurrently write the same `UnsafeCell<Frame>`, violating Rust's memory safety requirements. The first enabled callback could also allocate the ring: `ENABLED` became true before the lazy `ring()` allocation.

The corrected queue attempts producer ownership with one atomic compare-exchange. On contention it drops that measurement and increments the drop counter; it never spins, waits, allocates, or performs file I/O in the callback. Ownership transfers use acquire/release ordering, followed by the existing release/acquire slot publication and reclamation. Consumer ownership is likewise protected so test drains cannot introduce concurrent reads/reclamation. This is a bounded try-ownership design, not a claim of an always-successful lock-free queue.

Initialization serializes concurrent plugin lifecycle calls using a mutex that callbacks never access, allocates and touches the ring before enabling, and starts at most one writer. Callback publication uses `OnceLock::get`, never `get_or_init`. Writer errors disable recording. Test activation follows the same preallocation requirement, using a thread-local capture with the actual Ring implementation. This prevents unrelated host tests from publishing into each other’s fixtures or toggling each other’s activation. A four-thread test verifies capture isolation.

The CSV adds `dropped_total`, a cumulative writer-observed count of queue overflow and producer contention. It is not the count at each row's callback time. This remains a process-wide stream, without per-instance attribution. Sequence IDs are assigned at callback start; concurrently processed callbacks can publish out of order. Use one instance for instance-specific performance reports. Dropped measurements can bias percentiles and regressions; they cannot be silently treated as complete traces. Profiling overhead must itself be measured, and correlated route counters do not establish independent route costs.

## Reproducible checks

Run `RUSTC=/path/to/rustc python3 tools/audits/profiler_safety/check.py`.

The harness compiles **the actual production `src/cpu_profile.rs`**, without copying its implementation or replacing its atomics. On Rust 1.97.1:

- Ten module tests pass, including eight concurrent producers making 160,000 total publication attempts with a concurrent consumer. Every received frame has a unique ID and complete internally consistent fields. Successful pushes exactly match received frames; successful plus dropped frames exactly matches all attempts.
- A deliberately held producer owner causes immediate failure, increments drops, and leaves the queue head untouched; publication resumes after release.
- A separate process instruments the allocator around the disabled callback, first enabled callback, and queue saturation: **zero allocations, reallocations, or deallocations**. Ring allocation occurs in test lifecycle initialization before measurement. Saturation drops are counted and all retained records drain.
- A second allocator check compiles without `cfg(test)` and calls actual production `initialize()` with a real CSV writer. The first enabled and 16,384 sustained callbacks perform **zero allocator operations on the callback thread**. Writer-thread allocations are excluded deliberately.

These establish the tested module's correctness properties; a stress test is not a formal memory-model proof or a platform-wide timing bound. The full plugin build remains blocked by the unavailable private `derpcat-access` dependency in this environment. No full-plugin speedup or host callback percentile improvement is claimed by this safety fix.
