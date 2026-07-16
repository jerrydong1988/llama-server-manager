use std::ffi::{c_char, c_void, CStr};
#[cfg(target_os = "windows")]
use std::path::{Path, PathBuf};
use std::sync::Mutex;

type NvmlResult = u32;
const NVML_SUCCESS: NvmlResult = 0;
const NVML_ERROR_NO_PERMISSION: NvmlResult = 6;

// #3: Use Mutex instead of static mut to avoid data races, matching adlx.rs.
struct NvmlState {
    lib: &'static libloading::Library,
    initialized: bool,
}
unsafe impl Send for NvmlState {}
unsafe impl Sync for NvmlState {}

static NVML_STATE: Mutex<Option<NvmlState>> = Mutex::new(None);

#[cfg(target_os = "linux")]
const NVML_DLL: &str = "libnvidia-ml.so.1";

#[cfg(target_os = "windows")]
fn windows_nvml_candidates(system_root: &Path, program_files: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(2);
    if let Some(program_files) = program_files {
        candidates.push(program_files.join("NVIDIA Corporation/NVSMI/nvml.dll"));
    }
    candidates.push(system_root.join("System32/nvml.dll"));
    candidates
}

#[cfg(target_os = "windows")]
unsafe fn load_nvml_library() -> Option<libloading::Library> {
    let system_root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)?;
    let program_files = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from);
    windows_nvml_candidates(&system_root, program_files.as_deref())
        .into_iter()
        .find_map(|path| libloading::Library::new(path).ok())
}

#[cfg(target_os = "linux")]
unsafe fn load_nvml_library() -> Option<libloading::Library> {
    libloading::Library::new(NVML_DLL)
        .or_else(|_| libloading::Library::new("libnvidia-ml.so"))
        .ok()
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn ensure_loaded() -> bool {
    false
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn ensure_loaded() -> bool {
    let mut state = NVML_STATE.lock().unwrap();
    if state.is_some() {
        return true;
    }
    let Some(library) = (unsafe { load_nvml_library() }) else {
        return false;
    };
    let lib = Box::leak(Box::new(library));
    *state = Some(NvmlState {
        lib,
        initialized: false,
    });
    true
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
    pub gpu_name: Option<String>,
}

/// Collect GPU metrics via NVML. Returns None if NVIDIA driver/GPU not available.
pub fn collect_metrics() -> Option<NvmlMetrics> {
    if !ensure_loaded() {
        return None;
    }
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { try_collect() }));
    match result {
        Ok(r) => r,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "?".into()
            };
            eprintln!("[nvml] PANIC: {}", msg);
            None
        }
    }
}

unsafe fn try_collect() -> Option<NvmlMetrics> {
    let mut state_guard = NVML_STATE.lock().unwrap();
    let state = state_guard.as_mut()?;
    let lib = state.lib;

    // Initialize NVML once.
    if !state.initialized {
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
        state.initialized = true;
        eprintln!("[nvml] initialized OK");
    }

    type GetCountFn = unsafe extern "system" fn(*mut u32) -> NvmlResult;
    let get_count: libloading::Symbol<GetCountFn> = lib.get(b"nvmlDeviceGetCount_v2\0").ok()?;
    let mut device_count = 0;
    if get_count(&mut device_count) != NVML_SUCCESS || device_count == 0 {
        return None;
    }

    type GetHandleFn = unsafe extern "system" fn(u32, *mut *mut c_void) -> NvmlResult;
    let get_handle: libloading::Symbol<GetHandleFn> =
        lib.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?;
    let mut m = NvmlMetrics {
        gpu_percent: None,
        vram_used_mb: None,
        vram_total_mb: None,
        gpu_name: None,
    };

    type GetUtilFn = unsafe extern "system" fn(*mut c_void, *mut NvmlUtilization) -> NvmlResult;
    let get_util: libloading::Symbol<GetUtilFn> =
        lib.get(b"nvmlDeviceGetUtilizationRates\0").ok()?;
    type GetMemFn = unsafe extern "system" fn(*mut c_void, *mut NvmlMemory) -> NvmlResult;
    let get_mem: libloading::Symbol<GetMemFn> = lib.get(b"nvmlDeviceGetMemoryInfo\0").ok()?;
    type GetNameFn = unsafe extern "system" fn(*mut c_void, *mut c_char, u32) -> NvmlResult;
    let get_name = lib.get::<GetNameFn>(b"nvmlDeviceGetName\0").ok();

    let mut vram_used_bytes = 0_u64;
    let mut vram_total_bytes = 0_u64;
    let mut memory_samples = 0_u32;
    let mut gpu_names = Vec::new();
    for index in 0..device_count {
        let mut device: *mut c_void = std::ptr::null_mut();
        if get_handle(index, &mut device) != NVML_SUCCESS || device.is_null() {
            continue;
        }

        let mut util = NvmlUtilization { gpu: 0, memory: 0 };
        if get_util(device, &mut util) == NVML_SUCCESS {
            m.gpu_percent = Some(m.gpu_percent.unwrap_or(0.0).max(util.gpu as f32));
        }

        let mut mem = NvmlMemory {
            total: 0,
            free: 0,
            used: 0,
        };
        if get_mem(device, &mut mem) == NVML_SUCCESS {
            vram_total_bytes = vram_total_bytes.saturating_add(mem.total);
            vram_used_bytes = vram_used_bytes.saturating_add(mem.used);
            memory_samples += 1;
        }

        if let Some(get_name) = get_name.as_ref() {
            let mut name = [0 as c_char; 96];
            if get_name(device, name.as_mut_ptr(), name.len() as u32) == NVML_SUCCESS {
                let name = CStr::from_ptr(name.as_ptr())
                    .to_string_lossy()
                    .trim()
                    .to_string();
                if !name.is_empty() && !gpu_names.contains(&name) {
                    gpu_names.push(name);
                }
            }
        }
    }
    if memory_samples > 0 {
        m.vram_total_mb = Some((vram_total_bytes as f64) / (1024.0 * 1024.0));
        m.vram_used_mb = Some((vram_used_bytes as f64) / (1024.0 * 1024.0));
    }
    if !gpu_names.is_empty() {
        m.gpu_name = Some(gpu_names.join(" + "));
    }

    Some(m)
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn nvml_candidates_are_absolute_driver_locations() {
        let candidates = windows_nvml_candidates(
            Path::new(r"C:\Windows"),
            Some(Path::new(r"C:\Program Files")),
        );
        assert_eq!(
            candidates,
            vec![
                PathBuf::from(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll"),
                PathBuf::from(r"C:\Windows\System32\nvml.dll"),
            ]
        );
        assert!(candidates.iter().all(|path| path.is_absolute()));
    }
}

/// Cleanup NVML on app shutdown; best-effort and not mandatory.
pub fn shutdown() {
    if let Ok(mut state) = NVML_STATE.lock() {
        if let Some(s) = state.as_ref() {
            unsafe {
                type ShutdownFn = unsafe extern "system" fn() -> NvmlResult;
                if let Ok(shutdown_fn) = s.lib.get::<ShutdownFn>(b"nvmlShutdown\0") {
                    let _ = shutdown_fn();
                }
            }
        }
        *state = None;
    }
}
