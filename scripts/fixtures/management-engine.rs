//! Hermetic management fixture, not an inference engine or performance benchmark.
use std::{
    io::{Read, Write},
    net::TcpListener,
};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| matches!(arg.as_str(), "--version" | "--help" | "-h")) {
        let marker = std::env::current_exe().unwrap().with_file_name("probe-delay-ms");
        if let Ok(value) = std::fs::read_to_string(marker) {
            let millis = value.trim().parse::<u64>().unwrap().min(4000);
            std::thread::sleep(std::time::Duration::from_millis(millis));
        }
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("version: 7000 (abcdef123) v0.7.0");
        return;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("llama-server management test fixture\n--model FNAME\n--host HOST\n--port PORT\n--alias NAME\n--metrics\n--slots\n--props\n--api-key KEY\n--ctx-size N\n--threads N\n--n-gpu-layers N\n--parallel N\n--no-warmup\n--temp N\n--top-p N\n--mirostat-lr N\n--xtc-threshold N\n--adaptive-decay N");
        return;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };
    let port = value("--port").unwrap_or_else(|| "8080".into());
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    println!("srv init: server is listening on http://127.0.0.1:{port}");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        std::thread::spawn(move || {
            let mut bytes = [0u8; 65536];
            let Ok(count) = stream.read(&mut bytes) else {
                return;
            };
            let request = String::from_utf8_lossy(&bytes[..count]);
            let target = request.split_whitespace().nth(1).unwrap_or("/");
            let (kind, body) = if target.starts_with("/metrics") {
                ("text/plain","llamacpp:prompt_tokens_total 100\nllamacpp:tokens_predicted_total 20\nllamacpp:requests_processing 0\nllamacpp:predicted_tokens_seconds 10\nllamacpp:prompt_tokens_seconds 100\nllamacpp:n_decode_total 20\nllamacpp:n_tokens_max 100\nllamacpp:requests_deferred 0\nllamacpp:n_busy_slots_per_decode 1\n")
            } else if target.starts_with("/slots") {
                ("application/json", "[{\"id\":0,\"is_processing\":false}]")
            } else if target.starts_with("/v1/models") {
                (
                    "application/json",
                    "{\"data\":[{\"id\":\"fixture\",\"object\":\"model\"}]}",
                )
            } else if target.starts_with("/v1/chat/completions") {
                ("application/json","{\"id\":\"fixture\",\"object\":\"chat.completion\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"fixture-ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}")
            } else {
                (
                    "application/json",
                    "{\"status\":\"ok\",\"total_slots\":1,\"model_path\":\"fixture.gguf\"}",
                )
            };
            let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        });
    }
}
