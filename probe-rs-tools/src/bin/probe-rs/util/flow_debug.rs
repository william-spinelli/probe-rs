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

/// Fixed RTT channel used for the debug bridge.
pub const FLOW_DEBUG_RTT_UP_CHANNEL: u32 = 1;
pub const FLOW_DEBUG_RTT_DOWN_CHANNEL: u32 = 0;

pub struct FlowDebugBridge {
    up_channel: u32,
    socket_tx: mpsc::UnboundedSender<Vec<u8>>,
    socket_connected: Arc<AtomicBool>,
    task: JoinHandle<anyhow::Result<()>>,
}

impl FlowDebugBridge {
    pub fn rtt_channel_from_target(&self) -> u32 {
        self.up_channel
    }

    pub fn is_client_connected(&self) -> bool {
        self.socket_connected.load(Ordering::Relaxed)
    }

    pub fn forward_to_client(&self, bytes: &[u8]) {
        if !self.is_client_connected() {
            return;
        }

        // If the receiver was dropped, the monitor path should continue running.
        let _ = self.socket_tx.send(bytes.to_vec());
    }

    pub async fn new(
        session: SessionInterface,
        rtt_client: Key<RttClient>,
        port: u16,
    ) -> anyhow::Result<FlowDebugBridge> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .with_context(|| format!("Failed to bind debug socket on 127.0.0.1:{port}"))?;

        let (to_client, from_client) = mpsc::unbounded_channel();
        let socket_connected = Arc::new(AtomicBool::new(false));

        let task = tokio::spawn(run_debug_bridge(
            listener,
            session,
            rtt_client,
            from_client,
            Arc::clone(&socket_connected),
        ));

        Ok(FlowDebugBridge {
            up_channel: FLOW_DEBUG_RTT_UP_CHANNEL,
            socket_tx: to_client,
            socket_connected,
            task,
        })
    }

    pub fn shutdown(self) {
        self.task.abort();
    }
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
                .rtt_bridge_write(
                    rtt_client,
                    FLOW_DEBUG_RTT_DOWN_CHANNEL,
                    buffer[..len].to_vec(),
                )
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
