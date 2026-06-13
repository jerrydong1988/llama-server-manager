use std::ffi::c_void;

type NvmlResult = u32;
const NVML_SUCCESS: NvmlResult = 0;
const NVML_ERROR_NO_PERMISSION: NvmlResult = 6;

static mut NVML_LIB: Option<&'static libloading::Library> = None;
static mut NVML_INITIALIZED: bool = false;

#[cfg(target_os = "windows")]
const NVML_DLL: &str = "nvml.dll";

#[cfg(target_os = "linux")]
const NVML_DLL: &str = "libnvidia-ml.so.1";

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn ensure_loaded() -> bool { false }

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn ensure_loaded() -> bool {
    unsafe {
        if (*std::ptr::addr_of!(NVML_LIB)).is_some() { return true; }
        let lib = match libloading::Library::new(NVML_DLL) {
            Ok(l) => Box::leak(Box::new(l)),
            Err(_) => {
                // Try alternate name on Linux
                #[cfg(target_os = "linux")]
                {
                    let lib2 = libloading::Library::new("libnvidia-ml.so");
                    match lib2 {
                        Ok(l) => Box::leak(Box::new(l)),
                        Err(_) => return false,
                    }
                }
                #[cfg(not(target_os = "linux"))]
                return false;
            }
        };
        NVML_LIB = Some(lib);
        true
    }
}

#[repr(C)]
struct NvmlUtilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

pub struct NvmlMetrics {
    pub gpu_percent: Option<f32>,
    pub vram_used_mb: Option<f64>,
    pub vram_total_mb: Option<f64>,
}

/// Collect GPU metrics via NVML. Returns None if NVIDIA driver/GPU not available.
pub fn collect_metrics() -> Option<NvmlMetrics> {
    if !ensure_loaded() { return None; }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { try_collect() }));
    match result {
        Ok(r) => r,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<String>() { s.clone() }
                else if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
                else { "?" .into() };
            eprintln!("[nvml] PANIC: {}", msg);
            None
        }
    }
}

unsafe fn try_collect() -> Option<NvmlMetrics> {
    let lib = NVML_LIB.unwrap();

    // ── Initialize NVML (only once) ──
    if !NVML_INITIALIZED {
        eprintln!("[nvml] initializing...");
        type InitFn = unsafe extern "system" fn() -> NvmlResult;
        let init: libloading::Symbol<InitFn> = lib.get(b"nvmlInit_v2\0").ok()?;
        let rc = init();
        if rc == NVML_ERROR_NO_PERMISSION {
            eprintln!("[nvml] nvmlInit_v2 failed: ERROR_NO_PERMISSION (run as admin or add user to nvidia group)");
            return None;
        }
        if rc != NVML_SUCCESS {
            eprintln!("[nvml] nvmlInit_v2 failed with code {}", rc);
            return None;
        }
        NVML_INITIALIZED = true;
        eprintln!("[nvml] initialized OK");
    }

    // ── Get device handle for GPU 0 ──
    type GetHandleFn = unsafe extern "system" fn(u32, *mut *mut c_void) -> NvmlResult;
    let get_handle: libloading::Symbol<GetHandleFn> = lib.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?;

    let mut device: *mut c_void = std::ptr::null_mut();
    if get_handle(0, &mut device) != NVML_SUCCESS || device.is_null() {
        eprintln!("[nvml] no GPU found at index 0");
        // Don't return None — maybe GPU is present but index 0 failed? Fall through gracefully.
    }

    let mut m = NvmlMetrics {
        gpu_percent: None,
        vram_used_mb: None,
        vram_total_mb: None,
    };

    if !device.is_null() {
        // ── Utilization ──
        type GetUtilFn = unsafe extern "system" fn(*mut c_void, *mut NvmlUtilization) -> NvmlResult;
        let get_util: libloading::Symbol<GetUtilFn> = lib.get(b"nvmlDeviceGetUtilizationRates\0").ok()?;

        let mut util: NvmlUtilization = NvmlUtilization { gpu: 0, memory: 0 };
        if get_util(device, &mut util) == NVML_SUCCESS {
            m.gpu_percent = Some(util.gpu as f32);
        }

        // ── Memory Info ──
        type GetMemFn = unsafe extern "system" fn(*mut c_void, *mut NvmlMemory) -> NvmlResult;
        let get_mem: libloading::Symbol<GetMemFn> = lib.get(b"nvmlDeviceGetMemoryInfo\0").ok()?;

        let mut mem: NvmlMemory = NvmlMemory { total: 0, free: 0, used: 0 };
        if get_mem(device, &mut mem) == NVML_SUCCESS {
            m.vram_total_mb = Some((mem.total as f64) / (1024.0 * 1024.0));
            m.vram_used_mb = Some((mem.used as f64) / (1024.0 * 1024.0));
        }
    }

    Some(m)
}

/// Cleanup NVML — call on app shutdown (best-effort, not mandatory)
pub fn shutdown() {
    unsafe {
        if let Some(lib) = NVML_LIB {
            type ShutdownFn = unsafe extern "system" fn() -> NvmlResult;
            if let Ok(shutdown_fn) = lib.get::<ShutdownFn>(b"nvmlShutdown\0") {
                let _ = shutdown_fn();
            }
        }
    }
}
