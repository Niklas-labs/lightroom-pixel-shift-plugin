//! OS process high-water memory, not a sample of current/estimated allocations.
use serde::Serialize;
#[derive(Debug, Default, Serialize)]
pub struct Resources {
    pub peak_resident_bytes: Option<u64>,
    pub cpu_user_seconds: Option<f64>,
    pub cpu_system_seconds: Option<f64>,
    /// OS operation counters, not byte counts; cached IO may not increment them.
    pub input_block_operations: Option<u64>,
    pub output_block_operations: Option<u64>,
}
#[cfg(unix)]
pub fn snapshot() -> Resources {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage writes a correctly sized rusage on success; inspect it
    // only after the return code confirms initialization.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return Resources::default();
    }
    let usage = unsafe { usage.assume_init() };
    let peak = u64::try_from(usage.ru_maxrss).ok();
    #[cfg(target_os = "macos")]
    let bytes = peak;
    #[cfg(not(target_os = "macos"))]
    let bytes = peak.and_then(|n| n.checked_mul(1024));
    // Linux getrusage can retain a pre-exec image's high-water mark. VmHWM is
    // scoped to the current address space and avoids large-launcher pollution.
    #[cfg(target_os = "linux")]
    let bytes = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|n| n.parse::<u64>().ok())
                .and_then(|n| n.checked_mul(1024))
        })
        .or(bytes);
    Resources {
        peak_resident_bytes: bytes,
        cpu_user_seconds: Some(usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1e6),
        cpu_system_seconds: Some(
            usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1e6,
        ),
        input_block_operations: u64::try_from(usage.ru_inblock).ok(),
        output_block_operations: u64::try_from(usage.ru_oublock).ok(),
    }
}
#[cfg(windows)]
pub fn snapshot() -> Resources {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the current-process pseudo handle is valid and the writable struct
    // and supplied size match the Windows API contract. No handle is owned here.
    let size = counters.cb;
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    Resources {
        peak_resident_bytes: (ok != 0).then_some(counters.PeakWorkingSetSize as u64),
        ..Resources::default()
    }
}
#[cfg(not(any(unix, windows)))]
pub fn snapshot() -> Resources {
    Resources::default()
}
