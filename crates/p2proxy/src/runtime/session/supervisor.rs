use std::net::SocketAddr;
use std::time::Duration;

use futures::FutureExt;
use metrics::counter;
use tokio::net::TcpListener;
use tracing::{error, info};

use super::relay::{SessionContext, run_session};

/// Owns one listen port. Binds it, then supervises an accept loop that rebinds
/// on a fatal accept error or panic. Each accepted connection runs as an
/// independent panic-guarded task so one bad session never wedges the port.
pub struct SessionSupervisor;

impl SessionSupervisor {
    pub async fn spawn(ctx: SessionContext) -> std::io::Result<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], ctx.port));
        let listener = TcpListener::bind(addr).await?;
        counter!("p2proxy_socks_server_started_total").increment(1);
        info!(port = ctx.port, "SOCKS5 proxy listening");
        tokio::spawn(supervise(ctx, addr, listener));
        Ok(())
    }
}

async fn supervise(ctx: SessionContext, addr: SocketAddr, initial: TcpListener) {
    let mut listener = Some(initial);
    loop {
        let active = match listener.take() {
            Some(listener) => listener,
            None => match TcpListener::bind(addr).await {
                Ok(listener) => {
                    counter!("p2proxy_socks_listener_rebinds_total").increment(1);
                    info!(port = ctx.port, "SOCKS5 listener re-bound after crash");
                    listener
                }
                Err(e) => {
                    error!(port = ctx.port, ?e, "failed to re-bind, retrying in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            },
        };

        if std::panic::AssertUnwindSafe(accept_loop(&ctx, &active))
            .catch_unwind()
            .await
            .is_err()
        {
            counter!("p2proxy_socks_accept_loop_panics_total").increment(1);
            error!(port = ctx.port, "accept loop panicked — rebinding");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn accept_loop(ctx: &SessionContext, listener: &TcpListener) {
    loop {
        let (socket, _addr) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                counter!("p2proxy_socks_accept_errors_total").increment(1);
                error!(?e, "accept failed — rebinding listener");
                return;
            }
        };
        counter!("p2proxy_socks_connections_total").increment(1);
        let _ = socket.set_nodelay(true);
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if std::panic::AssertUnwindSafe(run_session(ctx, socket))
                .catch_unwind()
                .await
                .is_err()
            {
                counter!("p2proxy_socks_session_panics_total").increment(1);
                error!("SOCKS session panicked");
            }
        });
    }
}
