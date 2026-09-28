//! Per-session loopback transport. The child only proxies stdio; the desktop
//! retains the parsed capture. A random capability authenticates each connection.
use super::server::ProfilerServer;

/// MCP tool arguments are small; reject a peer's oversized line before the SDK
/// accumulates an unbounded JSON buffer. Responses have independent pagination.
struct LimitedLines<R> {
    inner: R,
    length: usize,
}
impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for LimitedLines<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let start = buf.filled().len();
        match std::pin::Pin::new(&mut self.inner).poll_read(cx, buf) {
            std::task::Poll::Ready(Ok(())) => {
                for byte in &buf.filled()[start..] {
                    if *byte == b'\n' {
                        self.length = 0;
                    } else {
                        self.length += 1;
                    }
                    if self.length > 65536 {
                        return std::task::Poll::Ready(Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "MCP request exceeds 64 KiB",
                        )));
                    }
                }
                std::task::Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
use rmcp::ServiceExt;
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

pub struct BridgeServer {
    address: SocketAddr,
    token: String,
    task: Option<JoinHandle<()>>,
}
impl BridgeServer {
    pub async fn start(server: ProfilerServer) -> std::io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let expected = token.clone();
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let Ok((mut socket,_)) = result else { break };
                        if clients.len() >= 8 { continue; }
                        let token = expected.clone();
                        let server = server.clone();
                        clients.spawn(async move {
                            let mut auth = [0u8;64];
                            let authorized = tokio::time::timeout(Duration::from_secs(3),socket.read_exact(&mut auth)).await;
                            if !matches!(authorized,Ok(Ok(_))) || auth.as_slice() != token.as_bytes() { return; }
                            if socket.write_all(b"OK").await.is_err() { return; }
                            let (read,write) = socket.into_split();
                            let serving = server.serve((LimitedLines {inner:read,length:0},write));
                            if let Ok(Ok(service)) = tokio::time::timeout(Duration::from_secs(15),serving).await {
                                let cancel = service.cancellation_token();
                                struct Guard(Option<rmcp::service::RunningServiceCancellationToken>);
                                impl Drop for Guard { fn drop(&mut self) { if let Some(token) = self.0.take() { token.cancel(); } } }
                                let _guard = Guard(Some(cancel));
                                let _ = service.waiting().await;
                            }
                        });
                    },
                    _ = clients.join_next(), if !clients.is_empty() => {},
                }
            }
        });
        Ok(Self {
            address,
            token,
            task: Some(task),
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn acp_config(&self, executable: &std::path::Path) -> serde_json::Value {
        serde_json::json!({"name":"unity-profiler","command":executable,"args":["--mcp-bridge",self.address.to_string()],
            "env":[{"name":"UPAA_MCP_TOKEN","value":self.token}]})
    }
    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}
impl Drop for BridgeServer {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub async fn proxy<R, W>(
    address: SocketAddr,
    token: &str,
    mut input: R,
    mut output: W,
) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use std::io::{Error, ErrorKind};
    if !address.ip().is_loopback()
        || token.len() != 64
        || !token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "invalid local MCP capability",
        ));
    }
    let mut socket =
        tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(address)).await??;
    socket.write_all(token.as_bytes()).await?;
    let mut ack = [0; 2];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut ack)).await??;
    if &ack != b"OK" {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "MCP authentication failed",
        ));
    }
    let (mut read, mut write) = socket.into_split();
    tokio::select! {
        result = tokio::io::copy(&mut input,&mut write) => { result?; },
        result = tokio::io::copy(&mut read,&mut output) => { result?; },
    }
    output.flush().await?;
    Ok(())
}

pub fn run_stdio(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    let address = address.parse()?;
    let token = std::env::var("UPAA_MCP_TOKEN")?;
    std::env::remove_var("UPAA_MCP_TOKEN");
    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(proxy(
        address,
        &token,
        tokio::io::stdin(),
        tokio::io::stdout(),
    ));
    // Tokio stdin may leave a blocking read alive after peer shutdown.
    runtime.shutdown_background();
    result?;
    Ok(())
}
