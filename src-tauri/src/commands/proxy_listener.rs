use std::io;
use std::net::SocketAddr;
use tokio::net::{lookup_host, TcpListener, TcpSocket, ToSocketAddrs};

/// Create the router listener without allowing model subprocesses to inherit it.
/// Mio's Windows bind path creates an inheritable socket. Closing that listener
/// in the manager then leaves the port held by any child launched while it ran.
pub(crate) async fn bind_proxy_listener(addr: impl ToSocketAddrs) -> io::Result<TcpListener> {
    let mut last_error = None;
    for addr in lookup_host(addr).await? {
        match bind_addr(addr) {
            Ok(listener) => return Ok(listener),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not resolve a listen address",
        )
    }))
}

fn bind_addr(addr: SocketAddr) -> io::Result<TcpListener> {
    // TcpSocket uses socket2, which creates Windows sockets with
    // WSA_FLAG_NO_HANDLE_INHERIT atomically (no race with concurrent launches).
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    // Preserve Tokio/Mio's Unix restart behavior. On Windows SO_REUSEADDR would
    // allow binding an actively used port, concealing a real listener conflict.
    #[cfg(not(windows))]
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(128)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn verify_release(host: &str) {
        let listener = bind_proxy_listener((host, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        assert_eq!(
            bind_proxy_listener(addr).await.unwrap_err().kind(),
            io::ErrorKind::AddrInUse
        );
        drop(client);
        drop(accepted);
        drop(listener);
        let rebound = bind_proxy_listener(addr).await.unwrap();
        assert_eq!(rebound.local_addr().unwrap(), addr);
    }

    #[tokio::test]
    async fn ipv4_listener_releases_port_and_preserves_conflict_detection() {
        verify_release("127.0.0.1").await;
    }

    #[tokio::test]
    async fn ipv6_listener_releases_port_and_preserves_conflict_detection() {
        verify_release("::1").await;
    }

    #[tokio::test]
    async fn listener_tries_remaining_resolved_addresses() {
        let occupied = bind_proxy_listener(("127.0.0.1", 0)).await.unwrap();
        let addresses = [
            occupied.local_addr().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
        ];
        let fallback = bind_proxy_listener(addresses.as_slice()).await.unwrap();
        assert_ne!(
            occupied.local_addr().unwrap(),
            fallback.local_addr().unwrap()
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn listener_handle_cannot_be_inherited_by_model_processes() {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};

        let listener = bind_proxy_listener(("127.0.0.1", 0)).await.unwrap();
        let mut flags = 0;
        // SAFETY: the listener keeps this socket handle valid for the call.
        let result =
            unsafe { GetHandleInformation(listener.as_raw_socket() as HANDLE, &mut flags) };
        assert_ne!(result, 0, "{}", io::Error::last_os_error());
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    }
}
