use std::ffi::c_void;
use std::sync::Mutex;

type AdlxResult = i32;
const ADLX_OK: AdlxResult = 0;

// #11: 用 Mutex 替代 static mut 消除数据竞争
struct AdlxState {
    lib: &'static libloading::Library,
    sys: *mut c_void,
}
unsafe impl Send for AdlxState {}
unsafe impl Sync for AdlxState {}

static ADLX_STATE: Mutex<Option<AdlxState>> = Mutex::new(None);

#[cfg(target_os = "windows")]
const ADLX_DLL: &str = r"C:\Windows\System32\amdadlx64.dll";

#[cfg(target_os = "linux")]
const ADLX_DLL: &str = "libADLX.so";

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn ensure_loaded() -> bool { false }

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn ensure_loaded() -> bool {
    let mut state = ADLX_STATE.lock().unwrap();
    if state.is_some() { return true; }
    let lib = unsafe { match libloading::Library::new(ADLX_DLL) {
        Ok(l) => Box::leak(Box::new(l)),
        Err(_) => return false,
    } };
    *state = Some(AdlxState { lib, sys: std::ptr::null_mut() });
    true
}

pub struct AdlxMetrics {
    pub cpu_percent: Option<f32>,
    pub memory_mb: Option<f64>,
    pub gpu_percent: Option<f32>,
    pub vram_used_mb: Option<f64>,
    pub vram_total_mb: Option<f64>,
}

/// Read function pointer from vtable at given index, cast to target type
unsafe fn vtbl<T>(base: *const c_void, idx: usize) -> T {
    let slot = (base as *const usize).add(idx);
    std::mem::transmute_copy::<usize, T>(&*slot)
}

/// Release COM object (vtable index 1 = Release)
unsafe fn release(obj: *mut c_void) {
    if obj.is_null() { return; }
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    vtbl::<Release>(*(obj as *const *const c_void), 1)(obj);
}

pub fn collect_metrics() -> Option<AdlxMetrics> {
    if !ensure_loaded() { return None; }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { try_collect() }));
    match result {
        Ok(r) => r,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<String>() { s.clone() }
                else if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
                else { "?".to_string() };
            eprintln!("[adlx] PANIC: {}", msg);
            None
        }
    }
}

unsafe fn try_collect() -> Option<AdlxMetrics> {
    let mut state_guard = ADLX_STATE.lock().unwrap();
    let state = state_guard.as_mut()?;
    let lib = state.lib;

    // Initialize ADLX singleton (only once)
    if state.sys.is_null() {
        eprintln!("[adlx] initializing...");
        type InitFn = unsafe extern "system" fn(u64, *mut *mut c_void) -> AdlxResult;
        let init: libloading::Symbol<InitFn> = lib.get(b"ADLXInitialize\0").ok()?;
        let mut sys: *mut c_void = std::ptr::null_mut();
        if init(0, &mut sys) != ADLX_OK || sys.is_null() {
            eprintln!("[adlx] ADLXInitialize failed");
            return None;
        }
        state.sys = sys;
        eprintln!("[adlx] initialized OK");
    }

    let sys = state.sys;
    let vt = *(sys as *const *const c_void);
    let mut m = AdlxMetrics { cpu_percent: None, memory_mb: None, gpu_percent: None, vram_used_mb: None, vram_total_mb: None };

    // GetGPUs (index 1)
    let mut gpu_list: *mut c_void = std::ptr::null_mut();
    let mut gpu: *mut c_void = std::ptr::null_mut();
    type GetGPUs = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> AdlxResult;
    if vtbl::<GetGPUs>(vt, 1)(sys, &mut gpu_list) == ADLX_OK && !gpu_list.is_null() {
        // TotalSystemRAM (index 10)
        type TotalRAM = unsafe extern "system" fn(*mut c_void, *mut u32) -> AdlxResult;
        let mut ram: u32 = 0;
        vtbl::<TotalRAM>(vt, 10)(sys, &mut ram);
        if ram > 0 { m.memory_mb = Some(ram as f64); }

        // GPU list vtable
        let vt_gl = *(gpu_list as *const *const c_void);

        // GPUList::At_GPUList (index 11) → get GPU[0]
        type AtGPU = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> AdlxResult;
        if vtbl::<AtGPU>(vt_gl, 11)(gpu_list, 0, &mut gpu) == ADLX_OK && !gpu.is_null() {
            let vt_gpu = *(gpu as *const *const c_void);

            // GPU::TotalVRAM (index 11)
            type TotalVRAM = unsafe extern "system" fn(*mut c_void, *mut u32) -> AdlxResult;
            let mut vram: u32 = 0;
            vtbl::<TotalVRAM>(vt_gpu, 11)(gpu, &mut vram);
            if vram > 0 { m.vram_total_mb = Some(vram as f64); }
        }
    }

    // GetPerformanceMonitoringServices (index 9)
    type GetPM = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> AdlxResult;
    let mut pm: *mut c_void = std::ptr::null_mut();
    if vtbl::<GetPM>(vt, 9)(sys, &mut pm) != ADLX_OK || pm.is_null() {
        // Release before returning
        release(gpu);
        release(gpu_list);
        return Some(m);
    }
    let vt_pm = *(pm as *const *const c_void);

    // PM::GetCurrentSystemMetrics (index 19)
    type GetSM = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> AdlxResult;
    let mut sm: *mut c_void = std::ptr::null_mut();
    if vtbl::<GetSM>(vt_pm, 19)(pm, &mut sm) == ADLX_OK && !sm.is_null() {
        let vt_sm = *(sm as *const *const c_void);

        // SM::CPUUsage (index 4)
        type CPUUsage = unsafe extern "system" fn(*mut c_void, *mut f64) -> AdlxResult;
        let mut cpu: f64 = 0.0;
        vtbl::<CPUUsage>(vt_sm, 4)(sm, &mut cpu);
        if cpu > 0.0 { m.cpu_percent = Some(cpu as f32); }

        // SM::SystemRAM (index 5)
        type SysRAM = unsafe extern "system" fn(*mut c_void, *mut i32) -> AdlxResult;
        let mut sram: i32 = 0;
        vtbl::<SysRAM>(vt_sm, 5)(sm, &mut sram);
        if sram > 0 { m.memory_mb = Some(sram as f64); }
    }

    // PM::GetCurrentGPUMetrics (index 18) for GPU[0]
    let mut gm: *mut c_void = std::ptr::null_mut();
    if !gpu.is_null() {
        type GetGM = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> AdlxResult;
        if vtbl::<GetGM>(vt_pm, 18)(pm, gpu, &mut gm) == ADLX_OK && !gm.is_null() {
            let vt_gm = *(gm as *const *const c_void);

            // GM::GPUUsage (index 4)
            type GPUUsage = unsafe extern "system" fn(*mut c_void, *mut f64) -> AdlxResult;
            let mut gpu_pct: f64 = 0.0;
            vtbl::<GPUUsage>(vt_gm, 4)(gm, &mut gpu_pct);
            if gpu_pct > 0.0 { m.gpu_percent = Some(gpu_pct as f32); }

            // GM::GPUVRAM (index 12)
            type GPUVram = unsafe extern "system" fn(*mut c_void, *mut i32) -> AdlxResult;
            let mut gv_mb: i32 = 0;
            vtbl::<GPUVram>(vt_gm, 12)(gm, &mut gv_mb);
            if gv_mb > 0 { m.vram_used_mb = Some(gv_mb as f64); }
        }
    }

    // Release all COM objects in reverse order
    release(gm);
    release(sm);
    release(pm);
    release(gpu);
    release(gpu_list);

    Some(m)
}
