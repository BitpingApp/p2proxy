// socks_intermediary.rs
use bitping_swarm::auth::Auth;
use bitping_tcp_proxy::{Session, SessionInit, SessionTransfer, TargetAddr, TCP_PROXY_PROTOCOL};
use color_eyre::eyre::{self, Result};
use futures::{
    io::{ReadHalf, WriteHalf},
    AsyncReadExt, AsyncWrite, AsyncWriteExt,
};
use libp2p::{identity::Keypair, PeerId, Stream, StreamProtocol};
use libp2p_stream as stream;
use socks5_impl::protocol::{
    handshake, Address, AsyncStreamOperation, AuthMethod, Reply, Request, Response,
};
use std::{
    borrow::Cow,
    io,
    net::{SocketAddrV4, ToSocketAddrs},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    select,
};
use tracing::{error, info};
use uuid::Uuid;

struct TargetWrapper(TargetAddr);

impl From<Address> for TargetWrapper {
    fn from(value: Address) -> Self {
        match value {
            Address::SocketAddress(socket_addr) => TargetWrapper(TargetAddr::Ip(socket_addr)),
            Address::DomainAddress(d, p) => TargetWrapper(TargetAddr::Domain(d, p)),
        }
    }
}

async fn read_length_prefixed(stream: &mut ReadHalf<Stream>) -> io::Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes);

    let mut data = vec![0u8; len as usize];
    stream.read_exact(&mut data).await?;
    Ok(data)
}

async fn write_length_prefixed(
    stream: &mut (impl AsyncWrite + Unpin),
    data: &[u8],
) -> io::Result<()> {
    let span = tracing::span!(
        tracing::Level::DEBUG,
        "write_length_prefixed",
        data_len = data.len()
    );
    let _enter = span.enter();

    tracing::debug!("starting length-prefixed write");

    let len = data.len() as u32;
    match stream.write_all(&len.to_le_bytes()).await {
        Ok(_) => tracing::debug!(length = len, "wrote length prefix"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to write length prefix");
            return Err(e);
        }
    }

    match stream.write_all(data).await {
        Ok(_) => tracing::debug!(bytes_written = data.len(), "wrote data payload"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to write data payload");
            return Err(e);
        }
    }

    match stream.flush().await {
        Ok(_) => tracing::debug!("flushed stream"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to flush stream");
            return Err(e);
        }
    }

    tracing::debug!("completed length-prefixed write");
    Ok(())
}

pub async fn run_socks_proxy(
    local_keypair: &'static Keypair,
    token: Cow<'_, str>,
    peer: PeerId,
    control: stream::Control,
) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:1080").await?;
    info!("SOCKS5 proxy listening on 127.0.0.1:1080");

    let mut connection_count = 0;

    loop {
        let (mut socket, addr) = listener.accept().await?;
        connection_count += 1;
        info!("Accepted connection {} from {}", connection_count, addr);

        let mut connection_control = control.clone();
        let connection_id = connection_count;

        let token = token.to_string();
        tokio::spawn(async move {
            // SOCKS5 Handshake
            let request = match handshake::Request::retrieve_from_async_stream(&mut socket).await {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to read initial handshake: {}", e);
                    return;
                }
            };

            if !request.evaluate_method(AuthMethod::NoAuth) {
                let response = handshake::Response::new(AuthMethod::NoAcceptableMethods);
                let _ = response.write_to_async_stream(&mut socket).await;
                return;
            }

            let response = handshake::Response::new(AuthMethod::NoAuth);
            if let Err(e) = response.write_to_async_stream(&mut socket).await {
                error!("Failed to write auth response: {}", e);
                return;
            }

            // Connection Request
            let request = match Request::retrieve_from_async_stream(&mut socket).await {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to read connection request: {}", e);
                    return;
                }
            };

            let target_addr: TargetWrapper = request.address.clone().into();
            let response = Response::new(Reply::Succeeded, request.address);
            if let Err(e) = response.write_to_async_stream(&mut socket).await {
                error!("Failed to write connection response: {}", e);
                return;
            }

            // Connect to peer
            let mut stream = match connection_control
                .open_stream(peer, TCP_PROXY_PROTOCOL)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    error!("Failed to open stream to peer: {}", e);
                    let response = Response::new(Reply::GeneralFailure, Address::unspecified());
                    let _ = response.write_to_async_stream(&mut socket).await;
                    return;
                }
            };

            let session_id = Uuid::new_v4().to_string();
            // Send target address to peer
            let session_init = Session::Init(SessionInit {
                id: session_id.clone(),
                target_addr: target_addr.0,
            });

            let request = Auth::new(session_init, local_keypair, token).unwrap();

            let session_init_bytes = postcard::to_stdvec(&request).unwrap();

            if let Err(e) = write_length_prefixed(&mut stream, &session_init_bytes).await {
                error!(?e, "Failed to write init message to peer");
                return;
            }

            let (mut socket_read, mut socket_write) = socket.split();
            let (mut stream_read, mut stream_write) = stream.split();

            let mut socket_buf = [0u8; 8192];
            let mut incoming_hasher = blake3::Hasher::new();
            let mut outgoing_hasher = blake3::Hasher::new();

            loop {
                select! {
                    result = socket_read.read(&mut socket_buf) => match result {
                        Ok(0) => break,
                        Ok(n) => {
                            let transfer = Session::Transfer(SessionTransfer {
                                id: session_id.clone(),
                                bytes: socket_buf[..n].to_vec(),
                            });

                            if let Ok(transfer_bytes) = postcard::to_stdvec(&transfer) {
                                if let Err(e) = write_length_prefixed(&mut stream_write, &transfer_bytes).await {
                                    error!("Failed to write to peer: {}", e);
                                    break;
                                }
                                outgoing_hasher.update(&socket_buf[..n]);
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from client: {}", e);
                            break;
                        }
                    },
                    result = read_length_prefixed(&mut stream_read) => match result {
                        Ok(data) => {
                            match postcard::from_bytes::<Session>(&data) {
                                Ok(Session::Transfer(transfer)) => {
                                    if transfer.id == session_id {
                                        if let Err(e) = socket_write.write_all(&transfer.bytes).await {
                                            error!("Failed to write to client: {}", e);
                                            break;
                                        }
                                        if let Err(e) = socket_write.flush().await {
                                            error!("Failed to flush client write: {}", e);
                                            break;
                                        }
                                        incoming_hasher.update(&transfer.bytes);
                                    }
                                }
                                Ok(Session::Error(err)) => {
                                    error!("Received error message: {}", err);
                                    break;
                                }
                                _ => {
                                    error!("Unexpected session type during transfer");
                                    break;
                                }
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                        Err(e) => {
                            error!("Failed to read from peer: {}", e);
                            break;
                        }
                    }
                }
            }

            let _ = stream_write.close().await;
            let _ = socket_write.shutdown().await;

            let incoming_hash = hex::encode(incoming_hasher.finalize().as_bytes());
            let outgoing_hash = hex::encode(outgoing_hasher.finalize().as_bytes());

            info!(session_id, incoming_hash, outgoing_hash, "Session finished");
        });
    }
}
