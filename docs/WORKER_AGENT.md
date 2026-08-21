# Secure Worker Agent

The Secure Worker Agent is the Phase 3 remote-execution boundary for Llama Server
Manager. It ships inside the existing `lsm` command on Windows, macOS, Linux
x86_64, and Linux ARM64. Single-node installations do not need an Agent and the
existing manual, local, and SSH Worker modes remain available.

## Security properties

- Control and RPC data use TLS. Enrollment pins the first certificate in the
  certificate file as the sole trust anchor, records its SHA-256 fingerprint,
  and verifies the certificate server name. Additional PEM certificates are not
  trusted as independent Agent identities.
- A 256-bit bearer token is read from a private file for every connection. Token
  contents are not stored in `workers.json`, accepted on the command line, or
  emitted by `init`, `inspect`, logs, audit events, or IPC responses.
- Protocol version 1 allows only `status`, `rpc_start`, `rpc_stop`, and `audit`.
  Unknown actions and unknown JSON fields are rejected.
- Remote requests cannot provide a program, argument, environment variable,
  working directory, device selector, or filesystem path. `rpc_start` can launch
  only the absolute `rpc-server` executable configured locally on the Worker,
  with the fixed `--host 127.0.0.1 --port <configured-port>` arguments. At most
  one child is owned by an Agent process.
- The RPC port is bound to loopback. The manager exposes a persisted loopback
  bridge and carries RPC bytes through a separately authenticated TLS tunnel.
- Protocol frames are limited to 64 KiB and the two listeners share a 64-
  connection in-flight limit. Repeated unauthenticated failures are aggregated
  instead of creating an attacker-controlled stream of audit writes.
- Audit records form a SHA-256 hash chain. Authenticated actions are recorded
  before execution and fail closed if the audit log cannot be persisted. The
  Agent refuses startup and audit
  reads if an existing record is malformed, reordered, removed from the middle,
  changed, or exceeds the 8 MiB operational limit.

This design authenticates the machines and protects traffic in transit. It does
not attest the remote operating system, GPU firmware, model weights, or
`rpc-server` binary. Protect the Worker host and distribute credentials through a
separate trusted channel.

The manager-side loopback bridge carries the raw llama.cpp RPC protocol. TCP
loopback is host-local, not per-user: every process on the manager host can reach
the bridge. Secure Agent mode therefore requires a dedicated manager account or
host with no untrusted local OS users. This deployment requirement is shown in a
native confirmation dialog before the manager reads or restricts the token.

## 1. Prepare TLS material

Create a certificate whose SAN contains the DNS name used by the manager. A
private CA is recommended. The following development example creates a
self-signed certificate; production operators should use their normal PKI.

```bash
openssl req -x509 -newkey rsa:3072 -sha256 -nodes -days 365 \
  -keyout worker-agent.key -out worker-agent.crt \
  -subj "/CN=worker.example.net" \
  -addext "subjectAltName=DNS:worker.example.net"
```

Keep the private key on the Worker. Copy only the certificate to the manager.

## 2. Initialize the Worker

All paths must be absolute. The executable must resolve to `rpc-server.exe` on
Windows or `rpc-server` on Unix.

```text
lsm worker-agent init \
  --config C:\\LSM-Agent\\worker-agent.json \
  --name GPU-Worker-01 \
  --control 0.0.0.0:7443 \
  --tunnel 0.0.0.0:7444 \
  --advertise-host worker.example.net \
  --tls-cert C:\\LSM-Agent\\worker-agent.crt \
  --tls-key C:\\LSM-Agent\\worker-agent.key \
  --rpc-binary C:\\llama.cpp\\rpc-server.exe \
  --rpc-port 50052
```

`init` creates the JSON configuration and a private `worker-agent.token` beside
it. It prints paths, public endpoints, Agent ID, and certificate fingerprint,
but never the token. It refuses to overwrite an existing configuration or token.
Optional `--token-file`, `--audit-file`, and `--rpc-log` arguments relocate those
files.

The generated configuration has a `devices` array. Operators may add inventory
entries with `device_type`, `name`, `vram_mb`, and `free_mb`; these are metadata
only and never become process arguments.

## 3. Run as a service

Run the foreground command under the operating system's service manager:

```text
lsm worker-agent serve --config C:\\LSM-Agent\\worker-agent.json
```

- Windows: create a service or a Task Scheduler entry under the dedicated Worker
  account, with no interactive window and restart-on-failure enabled.
- Linux: use a systemd unit with a dedicated user, `NoNewPrivileges=true`, a
  read-only application directory, and write access only to the configured log,
  audit, and token directory.
- macOS: use a LaunchDaemon or per-user LaunchAgent with `KeepAlive` enabled.

Allow inbound TCP only for the configured control and tunnel ports. Do not expose
the loopback RPC port.

## 4. Enroll in the manager

Securely copy the Agent certificate and token to private files on the manager.
Open **Cluster Management → Secure Agent** and enter:

- control host/port and tunnel host/port;
- the certificate SAN as TLS server name;
- absolute manager-local paths to the copied certificate and token;
- a stable loopback bridge port, or `0` to allocate one during first enrollment.

The manager verifies TLS, token, protocol version, Agent ID, and certificate
fingerprint before persisting the Worker. Persistence contains only public
connection metadata and the two credential paths. The chosen bridge port is
stable and is recreated after manager restarts. If that port is occupied,
restoration marks the Worker offline rather than silently selecting a new port
that would invalidate saved `--rpc` settings.

Enrollment first shows a native confirmation containing the canonical
certificate and token paths, both remote endpoints, TLS server name, and local
bridge. Only after approval does the backend read the exact 64-hex-character
token, reset/restrict its ACL to the current user, and connect to the displayed
endpoint.

Use **Start Agent RPC**, **Stop Agent RPC**, **Test Connection**, and **Load
Audit** from the Worker row. Removing an Agent Worker attempts to stop its remote
child, always closes the local bridge, and then removes the persisted metadata.

## Credential rotation and recovery

Rotate the remote token locally on the Worker:

```text
lsm worker-agent rotate-token --config C:\\LSM-Agent\\worker-agent.json
```

Securely replace the manager-local token copy at the already enrolled path. The
Agent and manager reload the file for every new request and tunnel, so neither
process needs a restart. Existing tunnels finish with the credential used at
their authenticated opening; new connections require the new token.

Use `lsm worker-agent inspect --config <absolute-path>` to recover public
enrollment metadata without exposing the token. Certificate rotation changes
the pinned identity and therefore requires re-enrollment with the new
certificate. Back up the Agent JSON, token, certificate, private key, and audit
log according to their sensitivity; never commit them to a repository.

The audit file is intentionally bounded at 8 MiB. When it reaches that limit,
the Agent refuses authenticated lifecycle and tunnel actions. Stop the Agent,
move the complete audit file to protected archival storage without editing it,
and restart the Agent to begin a new hash chain. Retain the archived chain and
the timestamp of the service restart together for continuity.

## CLI contract

```text
lsm worker-agent help
lsm worker-agent init ...
lsm worker-agent serve --config <absolute-path>
lsm worker-agent rotate-token --config <absolute-path>
lsm worker-agent inspect --config <absolute-path>
```

Exit code `0` means success, `1` means validation or runtime failure, and `2`
means non-Unicode CLI input. No subcommand accepts token contents.
