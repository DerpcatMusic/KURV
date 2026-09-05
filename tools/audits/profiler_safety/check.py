#!/usr/bin/env python3
"""Compile the actual production module; no copied ring or allocator implementation."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
RUSTC = os.environ.get('RUSTC', 'rustc')
SOURCE = ROOT / 'src/cpu_profile.rs'
with tempfile.TemporaryDirectory(prefix='kurv-profiler-') as tmp:
    tmp = Path(tmp)
    subprocess.run([RUSTC, '--edition=2024', '--test', str(SOURCE), '-o', str(tmp / 'tests')], check=True)
    subprocess.run([str(tmp / 'tests')], check=True)
    # Separate process: globals start untouched and no other test can initialize
    # the ring or change ENABLED while allocation counting is armed.
    harness = r'''
#![allow(dead_code, unused_imports)]
#[path = "SOURCE_PATH"]
mod cpu_profile;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
static ARMED: AtomicBool = AtomicBool::new(false);
static OPERATIONS: AtomicUsize = AtomicUsize::new(0);
struct CheckedAllocator;
unsafe impl GlobalAlloc for CheckedAllocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) { OPERATIONS.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if ARMED.load(Ordering::Relaxed) { OPERATIONS.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) { OPERATIONS.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static ALLOCATOR: CheckedAllocator = CheckedAllocator;
fn callback() {
    let mut p = cpu_profile::BlockProfile::begin(64, 1);
    p.enter(cpu_profile::Item::Render);
    p.count(cpu_profile::Item::RouteSerial, 64);
    p.finish(8);
}
fn main() {
    ARMED.store(true, Ordering::Relaxed);
    callback(); // Disabled, before initialization.
    ARMED.store(false, Ordering::Relaxed);
    assert_eq!(OPERATIONS.load(Ordering::Relaxed), 0);
    cpu_profile::enable_for_test(); // Lifecycle preinitialization.
    ARMED.store(true, Ordering::Relaxed);
    for _ in 0..8192 { callback(); } // First enabled callback, wrap and overflow.
    ARMED.store(false, Ordering::Relaxed);
    assert_eq!(OPERATIONS.load(Ordering::Relaxed), 0);
    cpu_profile::disable_for_test();
    assert_eq!(cpu_profile::drain_for_test().len(), 4096);
    assert_eq!(cpu_profile::dropped(), 4096);
    println!("actual profiler: no allocator operations in disabled, first-enabled or full-ring callbacks");
}
'''.replace('SOURCE_PATH', str(SOURCE))
    (tmp / 'alloc.rs').write_text(harness)
    subprocess.run([RUSTC, '--edition=2024', '--cfg', 'test', str(tmp / 'alloc.rs'), '-o', str(tmp / 'alloc')], check=True)
    subprocess.run([str(tmp / 'alloc')], check=True)
    # Also exercise the non-test global publication path after real lifecycle
    # initialization. Count only the callback thread: the CSV writer is allowed
    # to allocate, format, and perform I/O on its own thread.
    production = harness.split('fn main() {')[0]
    production = production.replace(
        'static ARMED: AtomicBool = AtomicBool::new(false);',
        'thread_local! { static ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }')
    production = production.replace('ARMED.load(Ordering::Relaxed)', 'ARMED.with(|armed| armed.get())')
    production += r'''
fn main() {
    ARMED.with(|armed| armed.set(true));
    callback();
    ARMED.with(|armed| armed.set(false));
    assert_eq!(OPERATIONS.load(Ordering::Relaxed), 0);
    cpu_profile::initialize();
    assert!(cpu_profile::enabled(), "production initialization failed");
    ARMED.with(|armed| armed.set(true));
    for _ in 0..16384 { callback(); }
    ARMED.with(|armed| armed.set(false));
    assert_eq!(OPERATIONS.load(Ordering::Relaxed), 0);
    println!("non-test production profiler: real initialization, first enabled and sustained callbacks have no allocator operations");
}
'''
    (tmp / 'production.rs').write_text(production)
    subprocess.run([RUSTC, '--edition=2024', str(tmp / 'production.rs'), '-o', str(tmp / 'production')], check=True)
    subprocess.run([str(tmp / 'production')], env={**os.environ, 'KURV_CPU_PROFILE': str(tmp / 'profile.csv')}, check=True)
