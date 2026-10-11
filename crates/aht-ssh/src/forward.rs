//! Port forwarding over an SSH connection: local (`ssh -L`), remote (`ssh -R`) and dynamic
//! (`ssh -D`, a SOCKS5 proxy).

use crate::client::{net, SshConnection};
use aht_core::{Error, Result};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// What to forward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwardSpec {
    /// Listen on this computer; each connection is opened from the server to `dest`.
    Local {
        bind: (String, u16),
        dest: (String, u16),
    },
    /// Ask the server to listen; each connection is opened from this computer to `dest`.
    Remote {
        bind: (String, u16),
        dest: (String, u16),
    },
    /// A SOCKS5 proxy on this computer; each connection is opened from the server.
    Dynamic { bind: (String, u16) },
}

/// A running forward. Dropping it stops listening; connections already open run until either
/// side closes them (or the SSH connection ends).
pub struct Forward {
    /// The port actually listened on (differs from the requested one when that was 0).
    pub port: u16,
    task: Option<JoinHandle<()>>,
    remote: Option<(Arc<SshConnection>, String, u32)>,
}

impl std::fmt::Debug for Forward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Forward").field("port", &self.port).finish()
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        if let Some((conn, host, port)) = self.remote.take() {
            conn.remote_forwards().remove(port);
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    let _ = conn.handle.cancel_tcpip_forward(host, port).await;
                });
            }
        }
    }
}

/// How long a local port taken by the previous session (a reconnect) is waited for.
const BIND_WAIT: Duration = Duration::from_secs(5);
/// How often a listener checks whether its SSH connection is gone.
const ALIVE_CHECK: Duration = Duration::from_secs(1);

async fn bind(host: &str, port: u16) -> Result<TcpListener> {
    let deadline = tokio::time::Instant::now() + BIND_WAIT;
    loop {
        match TcpListener::bind((host, port)).await {
            Ok(l) => return Ok(l),
            Err(e)
                if e.kind() == std::io::ErrorKind::AddrInUse
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(250)).await
            }
            Err(e) => {
                return Err(Error::Network(format!(
                    "cannot listen on {host}:{port}: {e}"
                )))
            }
        }
    }
}

impl SshConnection {
    /// Start a forward. Local listeners stop by themselves once this connection is gone.
    pub async fn forward(self: &Arc<Self>, spec: &ForwardSpec) -> Result<Forward> {
        match spec {
            ForwardSpec::Local { bind: b, dest } => {
                let listener = bind(&b.0, b.1).await?;
                let port = listener.local_addr().map_err(net)?.port();
                let dest = dest.clone();
                let task = self.serve(listener, move |conn, tcp, peer| {
                    let dest = dest.clone();
                    async move { local_conn(conn, tcp, peer, dest).await }
                });
                Ok(Forward {
                    port,
                    task: Some(task),
                    remote: None,
                })
            }
            ForwardSpec::Dynamic { bind: b } => {
                let listener = bind(&b.0, b.1).await?;
                let port = listener.local_addr().map_err(net)?.port();
                let task = self.serve(listener, socks_conn);
                Ok(Forward {
                    port,
                    task: Some(task),
                    remote: None,
                })
            }
            ForwardSpec::Remote { bind: b, dest } => {
                let got = self
                    .handle
                    .tcpip_forward(b.0.clone(), b.1 as u32)
                    .await
                    .map_err(|e| {
                        Error::Denied(format!(
                            "the server refused to listen on {}:{}: {e}",
                            b.0, b.1
                        ))
                    })?;
                // Servers answer 0 when the requested port was not 0.
                let port = if b.1 == 0 { got } else { b.1 as u32 };
                self.remote_forwards().insert(port, dest.clone());
                Ok(Forward {
                    port: port as u16,
                    task: None,
                    remote: Some((self.clone(), b.0.clone(), port)),
                })
            }
        }
    }

    /// Accept connections on `listener` until this SSH connection ends; each one is handled by
    /// `handle` on its own task.
    fn serve<F, Fut>(self: &Arc<Self>, listener: TcpListener, handle: F) -> JoinHandle<()>
    where
        F: Fn(Arc<SshConnection>, TcpStream, SocketAddr) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let conn = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(ALIVE_CHECK);
            loop {
                tokio::select! {
                    a = listener.accept() => match a {
                        Ok((tcp, peer)) => {
                            let fut = handle(conn.clone(), tcp, peer);
                            tokio::spawn(async move {
                                if let Err(e) = fut.await {
                                    tracing::debug!("forwarded connection from {peer}: {e}");
                                }
                            });
                        }
                        Err(e) => tracing::debug!("accept failed: {e}"),
                    },
                    _ = tick.tick() => if conn.is_closed() { break },
                }
            }
        })
    }

    async fn open_to(&self, host: &str, port: u16, peer: SocketAddr) -> Result<SshStream> {
        let ch = self
            .handle
            .channel_open_direct_tcpip(host, port as u32, peer.ip().to_string(), peer.port() as u32)
            .await
            .map_err(|e| Error::Network(format!("the server could not open {host}:{port}: {e}")))?;
        Ok(ch.into_stream())
    }
}

type SshStream = russh::ChannelStream<russh::client::Msg>;

async fn local_conn(
    conn: Arc<SshConnection>,
    mut tcp: TcpStream,
    peer: SocketAddr,
    dest: (String, u16),
) -> Result<()> {
    let mut ch = conn.open_to(&dest.0, dest.1, peer).await?;
    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut ch).await;
    Ok(())
}

/// SOCKS5 (RFC 1928), no authentication, CONNECT only.
async fn socks_conn(conn: Arc<SshConnection>, mut tcp: TcpStream, peer: SocketAddr) -> Result<()> {
    let bad = |what: &str| Error::Invalid(format!("SOCKS: {what}"));
    let mut head = [0u8; 2];
    tcp.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(bad("only SOCKS5 is supported"));
    }
    let mut methods = vec![0u8; head[1] as usize];
    tcp.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        tcp.write_all(&[5, 0xff]).await?;
        return Err(bad("the client needs authentication"));
    }
    tcp.write_all(&[5, 0]).await?;

    let mut req = [0u8; 4];
    tcp.read_exact(&mut req).await?;
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            tcp.read_exact(&mut a).await?;
            IpAddr::from(a).to_string()
        }
        3 => {
            let mut n = [0u8; 1];
            tcp.read_exact(&mut n).await?;
            let mut name = vec![0u8; n[0] as usize];
            tcp.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| bad("bad host name"))?
        }
        4 => {
            let mut a = [0u8; 16];
            tcp.read_exact(&mut a).await?;
            IpAddr::from(a).to_string()
        }
        _ => return Err(bad("unknown address type")),
    };
    let mut p = [0u8; 2];
    tcp.read_exact(&mut p).await?;
    let port = u16::from_be_bytes(p);
    // Replies carry a zero IPv4 bound address: clients ignore it for CONNECT.
    let reply = |code: u8| [5, code, 0, 1, 0, 0, 0, 0, 0, 0];
    if req[1] != 1 {
        tcp.write_all(&reply(7)).await?;
        return Err(bad("only CONNECT is supported"));
    }
    match conn.open_to(&host, port, peer).await {
        Ok(mut ch) => {
            tcp.write_all(&reply(0)).await?;
            let _ = tokio::io::copy_bidirectional(&mut tcp, &mut ch).await;
            Ok(())
        }
        Err(e) => {
            // 5: connection refused (the server could not reach it).
            let _ = tcp.write_all(&reply(5)).await;
            Err(e)
        }
    }
}

/// Destinations of the server's listening ports (`ssh -R`), by port: the client handler
/// connects each forwarded connection to its destination.
#[derive(Clone, Default)]
pub(crate) struct RemoteForwards(
    Arc<std::sync::Mutex<std::collections::HashMap<u32, (String, u16)>>>,
);

impl RemoteForwards {
    fn insert(&self, port: u32, dest: (String, u16)) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(port, dest);
    }

    fn remove(&self, port: u32) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&port);
    }

    pub(crate) fn get(&self, port: u32) -> Option<(String, u16)> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&port)
            .cloned()
    }
}

/// Connect a channel the server opened for a remote forward to its destination.
pub(crate) fn connect_forwarded(channel: russh::Channel<russh::client::Msg>, dest: (String, u16)) {
    tokio::spawn(async move {
        match TcpStream::connect((dest.0.as_str(), dest.1)).await {
            Ok(mut tcp) => {
                let mut ch = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut ch, &mut tcp).await;
            }
            Err(e) => {
                tracing::debug!("remote forward to {}:{}: {e}", dest.0, dest.1);
                let _ = channel.close().await;
            }
        }
    });
}
