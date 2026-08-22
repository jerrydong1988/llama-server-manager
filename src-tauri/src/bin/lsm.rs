#[cfg(debug_assertions)]
fn run_test_fixture_server(arguments: &[std::ffi::OsString]) -> Result<(), String> {
    use std::io::{Read, Write};

    let option = |name: &str| {
        arguments
            .windows(2)
            .find(|pair| pair[0] == name)
            .and_then(|pair| pair[1].to_str())
    };
    let port = option("--fixture-listen-port")
        .or_else(|| option("--port"))
        .ok_or_else(|| "fixture server requires --port".to_string())?
        .parse::<u16>()
        .map_err(|error| format!("fixture server port is invalid: {error}"))?;
    let exit_after = option("--exit-after-ms")
        .map(|value| {
            value
                .parse::<u64>()
                .map(std::time::Duration::from_millis)
                .map_err(|error| format!("fixture server exit delay is invalid: {error}"))
        })
        .transpose()?;
    let crash_once_marker = option("--crash-once-marker");
    let crash_once_requested = crash_once_marker.is_some();
    let crash_once = crash_once_marker
        .map(std::path::PathBuf::from)
        .map(|marker| {
            if !marker.is_absolute() {
                return Err("fixture server crash marker must be absolute".to_string());
            }
            if marker.exists() {
                return Ok(false);
            }
            std::fs::write(&marker, b"crash once")
                .map_err(|error| format!("fixture server crash marker failed: {error}"))?;
            Ok(true)
        })
        .transpose()?
        .unwrap_or(false);
    let started = std::time::Instant::now();
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|error| format!("fixture server bind failed: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("fixture server setup failed: {error}"))?;
    loop {
        if exit_after.is_some_and(|delay| started.elapsed() >= delay)
            && (!crash_once_requested || crash_once)
        {
            return Err("fixture server requested a test crash".to_string());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(1)));
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request);
                let body = br#"{"status":"ok"}"#;
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(headers.as_bytes())
                    .and_then(|_| stream.write_all(body))
                    .map_err(|error| format!("fixture server response failed: {error}"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(error) => return Err(format!("fixture server accept failed: {error}")),
        }
    }
}

fn main() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "worker-agent")
    {
        std::process::exit(llama_server_manager::worker_agent::run_cli(
            arguments.into_iter().skip(1),
        ));
    }

    #[cfg(debug_assertions)]
    {
        if arguments
            .first()
            .is_some_and(|argument| argument == "__test-fixture-server")
        {
            if let Err(error) = run_test_fixture_server(&arguments[1..]) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
    }

    if llama_server_manager::runtime_service::is_runtime_service_invocation() {
        if let Err(error) =
            llama_server_manager::runtime_service::configure_runtime_data_dir_from_args()
        {
            eprintln!("Runtime service configuration failed: {error}");
            std::process::exit(1);
        }
        if let Err(error) = llama_server_manager::runtime_service::run_runtime_service() {
            eprintln!("Runtime service failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    std::process::exit(llama_server_manager::headless_cli::run(arguments));
}
