//! Engines get a private hidden console on Windows. Ctrl+C is broadcast only
//! inside that console by a short-lived helper, never from the GUI/daemon.
#[cfg(not(windows))]
pub(crate) type Child = std::process::Child;
#[cfg(windows)]
pub(crate) use windows::Child;

pub(crate) fn spawn(args: &[String], cwd: Option<&str>) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        windows::spawn(args, cwd)
    }
    #[cfg(not(windows))]
    {
        let mut command = crate::process_environment::external_command(&args[0]);
        command
            .args(&args[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command.spawn()
    }
}

/// Use the effective argv, not configuration fields: manual/custom arguments
/// can also select a writable external cache. A prepared run replaces it with
/// an exclusively owned private file before launch.
pub(crate) fn safe_to_flush(args: &[String], private_cache: bool) -> bool {
    private_cache
        || !args.iter().any(|arg| {
            arg == "-lcd"
                || arg == "--lookup-cache-dynamic"
                || arg.starts_with("-lcd=")
                || arg.starts_with("--lookup-cache-dynamic=")
        })
}

pub(crate) fn termination_lock(pid: u32) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<u32, Weak<Mutex<()>>>>> =
        OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap();
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&pid).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(pid, Arc::downgrade(&lock));
    lock
}

pub(crate) fn helper_invocation() -> Option<i32> {
    #[cfg(windows)]
    {
        let args = std::env::args().collect::<Vec<_>>();
        if args.get(1).map(String::as_str) == Some("--signal-engine-console") {
            return Some(match windows::signal(&args[2..]) {
                Ok(()) => 0,
                Err(code) => code,
            });
        }
    }
    None
}

#[cfg(windows)]
pub(crate) fn request_exit(running: &crate::models::RunningInstance) -> bool {
    if !running.private_console {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    request_exit_with_helper(running, &exe)
}

#[cfg(windows)]
fn request_exit_with_helper(
    running: &crate::models::RunningInstance,
    exe: &std::path::Path,
) -> bool {
    use std::os::windows::process::CommandExt;
    let Ok(mut helper) = std::process::Command::new(exe)
        .args([
            "--signal-engine-console",
            &running.pid.to_string(),
            &running.start_time.to_string(),
            &running.executable_path,
        ])
        .creation_flags(0x08000000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match helper.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    eprintln!(
                        "engine console helper failed: pid={} status={status}",
                        running.pid
                    );
                }
                return status.success();
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            _ => {
                let _ = helper.kill();
                let _ = helper.wait();
                return false;
            }
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
    use std::os::windows::process::ExitStatusExt;
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::{Console::*, Pipes::CreatePipe, Threading::*};

    pub(crate) struct Child {
        process: OwnedHandle,
        pid: u32,
        pub stdout: Option<std::fs::File>,
        pub stderr: Option<std::fs::File>,
    }
    impl Child {
        pub fn id(&self) -> u32 {
            self.pid
        }
        pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
            unsafe {
                match WaitForSingleObject(self.process.as_raw_handle(), 0) {
                    WAIT_TIMEOUT => Ok(None),
                    WAIT_OBJECT_0 => {
                        let mut code = 0;
                        if GetExitCodeProcess(self.process.as_raw_handle(), &mut code) == 0 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(Some(std::process::ExitStatus::from_raw(code)))
                    }
                    _ => Err(io::Error::last_os_error()),
                }
            }
        }
        pub fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
            unsafe {
                if WaitForSingleObject(self.process.as_raw_handle(), INFINITE) != WAIT_OBJECT_0 {
                    return Err(io::Error::last_os_error());
                }
            }
            self.try_wait()?
                .ok_or_else(|| io::Error::other("process did not exit"))
        }
        pub fn kill(&mut self) -> io::Result<()> {
            unsafe {
                if TerminateProcess(self.process.as_raw_handle(), 1) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }

    fn wide(s: &OsStr) -> io::Result<Vec<u16>> {
        let mut value = s.encode_wide().collect::<Vec<_>>();
        if value.contains(&0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "embedded NUL"));
        }
        value.push(0);
        Ok(value)
    }

    // MS CRT quoting, with no shell involved (including embedded quotes and trailing slashes).
    fn quote(arg: &str) -> String {
        // Keep simple flags bare, matching std::process::Command. In particular
        // cmd.exe parses /D, /S and /C itself rather than with the MS CRT parser.
        if !arg.is_empty() && !arg.chars().any(|c| matches!(c, ' ' | '\t' | '"')) {
            return arg.to_owned();
        }
        let mut out = String::from("\"");
        let mut slashes = 0;
        for c in arg.chars() {
            if c == '\\' {
                slashes += 1;
                continue;
            }
            out.extend(std::iter::repeat('\\').take(if c == '"' {
                slashes * 2 + 1
            } else {
                slashes
            }));
            out.push(c);
            slashes = 0;
        }
        out.extend(std::iter::repeat('\\').take(slashes * 2));
        out.push('"');
        out
    }

    fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        unsafe {
            if CreatePipe(&mut read, &mut write, &sa, 0) == 0 {
                return Err(io::Error::last_os_error());
            }
            let read = OwnedHandle::from_raw_handle(read);
            let write = OwnedHandle::from_raw_handle(write);
            if SetHandleInformation(read.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok((read, write))
        }
    }

    // Some Windows ROCm runtimes append forward-slash library paths to the
    // loaded executable path. That is invalid under Win32 verbatim semantics.
    // Use an ordinary short drive path only when canonicalization proves it
    // identifies the same file; retain long/device/UNC paths unchanged.
    fn launch_path(path: &str) -> String {
        if let Some(short) = path.strip_prefix(r"\\?\") {
            if short.as_bytes().get(1) == Some(&b':')
                && short.as_bytes().get(2) == Some(&b'\\')
                && short.encode_utf16().count() < 260
                && std::fs::canonicalize(short)
                    .ok()
                    .zip(std::fs::canonicalize(path).ok())
                    .is_some_and(|(left, right)| left == right)
            {
                return short.to_owned();
            }
        }
        path.to_owned()
    }

    pub(super) fn spawn(args: &[String], cwd: Option<&str>) -> io::Result<Child> {
        let program = args
            .first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty engine command"))?;
        if !std::path::Path::new(program).is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "engine executable must be absolute",
            ));
        }
        let program = launch_path(program);
        let application = wide(OsStr::new(&program))?;
        let mut command = wide(OsStr::new(
            &std::iter::once(&program)
                .chain(args[1..].iter())
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(" "),
        ))?;
        let directory = cwd.map(|s| wide(OsStr::new(&launch_path(s)))).transpose()?;
        let (stdout, out_write) = pipe()?;
        let (stderr, err_write) = pipe()?;
        let input = std::fs::File::open("NUL")?;
        unsafe {
            if SetHandleInformation(
                input.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            // An explicit handle list prevents inheriting router sockets/other instances' pipes.
            let handles = [
                input.as_raw_handle(),
                out_write.as_raw_handle(),
                err_write.as_raw_handle(),
            ];
            let mut bytes = 0;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
            let mut storage = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
            let attributes = storage.as_mut_ptr().cast();
            if InitializeProcThreadAttributeList(attributes, 1, 0, &mut bytes) == 0 {
                return Err(io::Error::last_os_error());
            }
            let result = (|| {
                if UpdateProcThreadAttribute(
                    attributes,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    std::mem::size_of_val(&handles),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let mut startup: STARTUPINFOEXW = std::mem::zeroed();
                startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
                startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
                startup.StartupInfo.wShowWindow = 0; // SW_HIDE from the first frame.
                startup.StartupInfo.hStdInput = handles[0];
                startup.StartupInfo.hStdOutput = handles[1];
                startup.StartupInfo.hStdError = handles[2];
                startup.lpAttributeList = attributes;
                let mut info: PROCESS_INFORMATION = std::mem::zeroed();
                if CreateProcessW(
                    application.as_ptr(),
                    command.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                    CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                    std::ptr::null(),
                    directory.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                    &startup.StartupInfo,
                    &mut info,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let _thread = OwnedHandle::from_raw_handle(info.hThread);
                Ok(Child {
                    process: OwnedHandle::from_raw_handle(info.hProcess),
                    pid: info.dwProcessId,
                    stdout: Some(std::fs::File::from_raw_handle(stdout.into_raw_handle())),
                    stderr: Some(std::fs::File::from_raw_handle(stderr.into_raw_handle())),
                })
            })();
            DeleteProcThreadAttributeList(attributes);
            result
        }
    }

    pub(super) fn signal(args: &[String]) -> Result<(), i32> {
        if args.len() != 3 {
            return Err(2);
        }
        let pid = args[0].parse::<u32>().map_err(|_| 2)?;
        let expected = args[1].parse::<u64>().map_err(|_| 2)?;
        let identity_matches = || {
            crate::commands::server::read_process_identity(pid).is_some_and(|(start, path)| {
                start == expected
                    && crate::path_utils::paths_equal(&path, std::path::Path::new(&args[2]))
            })
        };
        if !identity_matches() {
            return Err(3);
        }
        unsafe {
            FreeConsole();
            if AttachConsole(pid) == 0 {
                return Err(4);
            }
            let result = (|| {
                let mut members = [0u32; 3];
                let count = GetConsoleProcessList(members.as_mut_ptr(), members.len() as u32);
                if count != 2
                    || !members[..2].contains(&pid)
                    || !members[..2].contains(&std::process::id())
                {
                    return Err(5);
                }
                if !identity_matches() {
                    return Err(3);
                }
                if SetConsoleCtrlHandler(None, 1) == 0 {
                    return Err(6);
                }
                if GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) == 0 {
                    return Err(7);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                Ok(())
            })();
            FreeConsole();
            result
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn short_verbatim_launch_path_keeps_the_verified_file_identity() {
            let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
            let raw = exe.to_string_lossy();
            let ordinary = launch_path(&raw);
            assert_eq!(std::fs::canonicalize(&ordinary).unwrap(), exe);
            if raw
                .strip_prefix(r"\\?\")
                .is_some_and(|s| s.encode_utf16().count() < 260)
            {
                assert!(!ordinary.starts_with(r"\\?\"));
            }
            let long = format!(r"\\?\C:\{}\engine.exe", "x".repeat(260));
            assert_eq!(launch_path(&long), long);
            assert_eq!(
                launch_path(r"\\?\UNC\server\share\engine.exe"),
                r"\\?\UNC\server\share\engine.exe"
            );
        }

        #[test]
        fn quotes_empty_unicode_embedded_quotes_and_trailing_slashes() {
            assert_eq!(quote("/C"), "/C");
            assert_eq!(quote(""), "\"\"");
            assert_eq!(quote("a\"b"), "\"a\\\"b\"");
            assert_eq!(quote("缓存 path\\"), "\"缓存 path\\\\\"");
        }

        #[test]
        #[ignore = "requires LSM_TEST_HELPER_EXE and LSM_TEST_PYTHON; spawns only isolated signal fixtures"]
        fn private_console_signal_checks_identity_and_preserves_other_process() {
            use std::io::{BufRead, BufReader};
            let helper = std::path::PathBuf::from(std::env::var_os("LSM_TEST_HELPER_EXE").unwrap());
            let python = std::env::var("LSM_TEST_PYTHON").unwrap();
            struct Owned(Child);
            impl Drop for Owned {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            let start = || {
                let command = vec![python.clone(), "-u".into(), "-c".into(),
                    "import signal,time,sys; signal.signal(signal.SIGINT, lambda *_: sys.exit(0)); print('ready'); time.sleep(60)".into()];
                let mut child = Owned(spawn(&command, None).unwrap());
                let mut ready = String::new();
                BufReader::new(child.0.stdout.take().unwrap())
                    .read_line(&mut ready)
                    .unwrap();
                assert_eq!(ready.trim(), "ready");
                child
            };
            let mut target = start();
            let mut other = start();
            let (start_time, executable_path) =
                crate::commands::server::read_process_identity(target.0.id()).unwrap();
            let mut running: crate::models::RunningInstance = serde_json::from_value(serde_json::json!({
                "instance_id":"console-fixture", "pid":target.0.id(), "port":0, "host":"127.0.0.1",
                "private_console":true, "start_time":start_time + 1, "executable_path":executable_path
            })).unwrap();
            assert!(!super::super::request_exit_with_helper(&running, &helper));
            assert!(target.0.try_wait().unwrap().is_none());
            running.start_time = start_time;
            assert!(super::super::request_exit_with_helper(&running, &helper));
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while target.0.try_wait().unwrap().is_none() && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            assert!(target
                .0
                .try_wait()
                .unwrap()
                .is_some_and(|status| status.success()));
            assert!(other.0.try_wait().unwrap().is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn external_dynamic_arguments_never_gain_implicit_exit_writes() {
        for flag in [
            "-lcd",
            "--lookup-cache-dynamic",
            "--lookup-cache-dynamic=cache",
            "-lcd=cache",
        ] {
            let args = vec!["engine".into(), flag.into(), "external.bin".into()];
            assert!(!super::safe_to_flush(&args, false));
            assert!(super::safe_to_flush(&args, true));
        }
        assert!(super::safe_to_flush(&["engine".into()], false));
    }
}
