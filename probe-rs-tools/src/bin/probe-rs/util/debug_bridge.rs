use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Context;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
};

use crate::{
    rpc::{Key, client::SessionInterface},
    util::rtt::client::RttClient,
};

/// Fixed RTT channel used for the debug protocol bridge.
pub const DEBUG_RTT_UP_CHANNEL: u32 = 1;
pub const DEBUG_RTT_DOWN_CHANNEL: u32 = 0;

#[derive(Clone)]
pub struct DebugRttForwarder {
    rtt_channel: u32,
    to_socket: mpsc::UnboundedSender<Vec<u8>>,
    socket_connected: Arc<AtomicBool>,
}

impl DebugRttForwarder {
    pub fn rtt_channel(&self) -> u32 {
        self.rtt_channel
    }

    pub fn is_connected(&self) -> bool {
        self.socket_connected.load(Ordering::Relaxed)
    }

    pub fn forward_to_socket(&self, bytes: &[u8]) {
        if !self.is_connected() {
            return;
        }

        // If the receiver was dropped, the monitor path should continue running.
        let _ = self.to_socket.send(bytes.to_vec());
    }
}

pub async fn start_debug_bridge(
    session: SessionInterface,
    rtt_client: Key<RttClient>,
    port: u16,
) -> anyhow::Result<(DebugRttForwarder, JoinHandle<anyhow::Result<()>>)> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("Failed to bind debug socket on 127.0.0.1:{port}"))?;

    let (to_socket, socket_rx) = mpsc::unbounded_channel();
    let socket_connected = Arc::new(AtomicBool::new(false));

    let forwarder = DebugRttForwarder {
        rtt_channel: DEBUG_RTT_UP_CHANNEL,
        to_socket,
        socket_connected: Arc::clone(&socket_connected),
    };

    let task = tokio::spawn(run_debug_bridge(
        listener,
        session,
        rtt_client,
        socket_rx,
        socket_connected,
    ));

    Ok((forwarder, task))
}

async fn run_debug_bridge(
    listener: TcpListener,
    session: SessionInterface,
    rtt_client: Key<RttClient>,
    mut socket_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    socket_connected: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    println!("Waiting for debug client on {}", listener.local_addr()?);
    let (stream, addr) = listener.accept().await?;
    println!("Debug client connected: {addr}");

    socket_connected.store(true, Ordering::Relaxed);

    let (mut read_stream, mut write_stream) = stream.into_split();

    let read_task = async {
        let mut buffer = [0_u8; 4096];

        loop {
            let len = read_stream.read(&mut buffer).await?;
            if len == 0 {
                return Ok::<(), anyhow::Error>(());
            }

            println!("read_stream: {:?}", &buffer[..len]);

            session
                .write_rtt_down_channel(rtt_client, DEBUG_RTT_DOWN_CHANNEL, buffer[..len].to_vec())
                .await?;
        }
    };

    let write_task = async {
        while let Some(bytes) = socket_rx.recv().await {
            write_stream.write_all(&bytes).await?;
        }
        Ok::<(), anyhow::Error>(())
    };

    let result = tokio::select! {
        result = read_task => result,
        result = write_task => result,
    };

    socket_connected.store(false, Ordering::Relaxed);
    tracing::info!("Debug client disconnected");

    result
}
