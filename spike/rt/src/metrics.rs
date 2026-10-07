//! Collection: process RSS and CPU from procfs, an allocation counter, and
//! CSV rows with a markdown summary beside them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};

/// RSS (KiB) and CPU time (clock ticks, user + system) of a process.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcSample {
    pub rss_kib: u64,
    pub cpu_ticks: u64,
}

/// Samples `/proc/<pid>`.
pub fn proc_sample(pid: u32) -> Result<ProcSample> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
    let rss_kib = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| anyhow!("no VmRSS for {pid}"))?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // Fields after the parenthesised command name; utime and stime are the
    // 14th and 15th fields of the whole line.
    let after = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or("");
    let f: Vec<&str> = after.split_whitespace().collect();
    let tick = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    Ok(ProcSample { rss_kib, cpu_ticks: tick(11) + tick(12) })
}

/// A global allocator that counts allocations and bytes.
pub struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// (allocations, bytes) since start.
#[must_use]
pub fn allocations() -> (u64, u64) {
    (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed))
}

/// Appends one row to a CSV file, writing the header when the file is new.
pub fn csv_row(path: &Path, header: &[&str], row: &[String]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let new = !path.exists();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    if new {
        writeln!(f, "{}", header.join(","))?;
    }
    let esc: Vec<String> = row
        .iter()
        .map(|v| if v.contains(',') || v.contains('"') { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.clone() })
        .collect();
    writeln!(f, "{}", esc.join(","))?;
    Ok(())
}
