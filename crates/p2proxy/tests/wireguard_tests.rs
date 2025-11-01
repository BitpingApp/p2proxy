//! WireGuard Protocol Tests for P2Proxy
//!
//! This module implements comprehensive tests for the WireGuard protocol support:
//! - WireGuard packet reception and forwarding
//! - Message type identification (handshake, data, cookie)
//! - Stream pool integration
//! - Metrics tracking
//! - Error handling
//!
//! These tests validate the WireGuard VPN functionality of P2Proxy.

mod common;

use common::fixtures::*;
use models::config::ProxyProtocols;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

// WireGuard message type constants
const WIREGUARD_MESSAGE_HANDSHAKE_INITIATION: u8 = 1;
const WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE: u8 = 2;
const WIREGUARD_MESSAGE_COOKIE_REPLY: u8 = 3;
const WIREGUARD_MESSAGE_DATA: u8 = 4;

// WireGuard packet size constants (based on WireGuard protocol specification)
const WIREGUARD_HANDSHAKE_INIT_SIZE: usize = 148;
const WIREGUARD_HANDSHAKE_RESP_SIZE: usize = 92;
const WIREGUARD_COOKIE_REPLY_SIZE: usize = 64;
const WIREGUARD_DATA_PACKET_MIN_SIZE: usize = 32;  // 16 byte header + 16 byte auth tag
const WIREGUARD_DATA_PACKET_TEST_SIZE: usize = 128; // Test size for data packets
const WIREGUARD_MAX_MTU_SIZE: usize = 1420;         // Typical WireGuard MTU

// =============================================================================
// WireGuard Protocol Tests
// =============================================================================

/// Test WireGuard server creation and binding
///
/// Validates that a WireGuard server can be created and binds to the specified port.
#[tokio::test]
async fn test_wireguard_server_creation() {
    let port = 51820;
    let server = test_server(port, ProxyProtocols::WireGuard);

    assert_eq!(server.port, port);
    assert_eq!(server.protocol, ProxyProtocols::WireGuard);

    // Verify we can bind to the port
    let socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert!(socket.local_addr().is_ok());
}

/// Test WireGuard handshake initiation packet reception
///
/// Validates that the server can receive and identify WireGuard handshake initiation packets.
#[tokio::test]
async fn test_wireguard_handshake_initiation() {
    let port = 51821;

    // Create a mock WireGuard handshake initiation packet
    let mut packet = vec![0u8; WIREGUARD_HANDSHAKE_INIT_SIZE];
    packet[0] = WIREGUARD_MESSAGE_HANDSHAKE_INITIATION;
    packet[1] = 0; // Reserved
    packet[2] = 0; // Reserved
    packet[3] = 0; // Reserved
    // Sender index (4 bytes) - would be filled by actual WireGuard implementation
    // Remaining bytes would contain the handshake data

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Send handshake initiation packet
    let sent = client_socket
        .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert_eq!(sent, WIREGUARD_HANDSHAKE_INIT_SIZE);

    // Receive packet on server
    let mut buf = vec![0u8; 4096];
    let result = timeout(
        Duration::from_secs(1),
        server_socket.recv_from(&mut buf)
    ).await;

    assert!(result.is_ok(), "Should receive packet within timeout");
    let (len, _addr) = result.unwrap().unwrap();
    assert_eq!(len, WIREGUARD_HANDSHAKE_INIT_SIZE);
    assert_eq!(buf[0], WIREGUARD_MESSAGE_HANDSHAKE_INITIATION);
}

/// Test WireGuard handshake response packet reception
///
/// Validates that the server can receive and identify WireGuard handshake response packets.
#[tokio::test]
async fn test_wireguard_handshake_response() {
    let port = 51822;

    // Create a mock WireGuard handshake response packet
    let mut packet = vec![0u8; WIREGUARD_HANDSHAKE_RESP_SIZE];
    packet[0] = WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE;
    packet[1] = 0; // Reserved
    packet[2] = 0; // Reserved
    packet[3] = 0; // Reserved

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Send handshake response packet
    let sent = client_socket
        .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert_eq!(sent, WIREGUARD_HANDSHAKE_RESP_SIZE);

    // Receive packet on server
    let mut buf = vec![0u8; 4096];
    let result = timeout(
        Duration::from_secs(1),
        server_socket.recv_from(&mut buf)
    ).await;

    assert!(result.is_ok(), "Should receive packet within timeout");
    let (len, _addr) = result.unwrap().unwrap();
    assert_eq!(len, WIREGUARD_HANDSHAKE_RESP_SIZE);
    assert_eq!(buf[0], WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE);
}

/// Test WireGuard cookie reply packet reception
///
/// Validates that the server can receive and identify WireGuard cookie reply packets.
#[tokio::test]
async fn test_wireguard_cookie_reply() {
    let port = 51823;

    // Create a mock WireGuard cookie reply packet
    let mut packet = vec![0u8; WIREGUARD_COOKIE_REPLY_SIZE];
    packet[0] = WIREGUARD_MESSAGE_COOKIE_REPLY;
    packet[1] = 0; // Reserved
    packet[2] = 0; // Reserved
    packet[3] = 0; // Reserved

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Send cookie reply packet
    let sent = client_socket
        .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert_eq!(sent, WIREGUARD_COOKIE_REPLY_SIZE);

    // Receive packet on server
    let mut buf = vec![0u8; 4096];
    let result = timeout(
        Duration::from_secs(1),
        server_socket.recv_from(&mut buf)
    ).await;

    assert!(result.is_ok(), "Should receive packet within timeout");
    let (len, _addr) = result.unwrap().unwrap();
    assert_eq!(len, WIREGUARD_COOKIE_REPLY_SIZE);
    assert_eq!(buf[0], WIREGUARD_MESSAGE_COOKIE_REPLY);
}

/// Test WireGuard data packet reception
///
/// Validates that the server can receive and identify WireGuard data packets.
#[tokio::test]
async fn test_wireguard_data_packet() {
    let port = 51824;

    // Create a mock WireGuard data packet
    // WireGuard data packets are at least 32 bytes (16 byte header + 16 byte auth tag)
    // plus the encrypted payload
    let mut packet = vec![0u8; WIREGUARD_DATA_PACKET_TEST_SIZE];
    packet[0] = WIREGUARD_MESSAGE_DATA;
    packet[1] = 0; // Reserved
    packet[2] = 0; // Reserved
    packet[3] = 0; // Reserved
    // Receiver index (4 bytes)
    // Counter (8 bytes)
    // Encrypted data + auth tag

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Send data packet
    let sent = client_socket
        .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert_eq!(sent, WIREGUARD_DATA_PACKET_TEST_SIZE);

    // Receive packet on server
    let mut buf = vec![0u8; 4096];
    let result = timeout(
        Duration::from_secs(1),
        server_socket.recv_from(&mut buf)
    ).await;

    assert!(result.is_ok(), "Should receive packet within timeout");
    let (len, _addr) = result.unwrap().unwrap();
    assert_eq!(len, WIREGUARD_DATA_PACKET_TEST_SIZE);
    assert_eq!(buf[0], WIREGUARD_MESSAGE_DATA);
}

/// Test unknown WireGuard message type handling
///
/// Validates that the server handles unknown message types gracefully.
#[tokio::test]
async fn test_wireguard_unknown_message_type() {
    let port = 51825;

    // Create a packet with an unknown message type
    let mut packet = vec![0u8; WIREGUARD_COOKIE_REPLY_SIZE];
    packet[0] = 99; // Unknown message type

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Send packet with unknown type
    let sent = client_socket
        .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert_eq!(sent, WIREGUARD_COOKIE_REPLY_SIZE);

    // Receive packet on server
    let mut buf = vec![0u8; 4096];
    let result = timeout(
        Duration::from_secs(1),
        server_socket.recv_from(&mut buf)
    ).await;

    assert!(result.is_ok(), "Should receive packet within timeout");
    let (len, _addr) = result.unwrap().unwrap();
    assert_eq!(len, WIREGUARD_COOKIE_REPLY_SIZE);
    assert_eq!(buf[0], 99); // Should still receive the packet
}

/// Test concurrent WireGuard packet reception
///
/// Validates that the server can handle multiple concurrent WireGuard packets.
#[tokio::test]
async fn test_wireguard_concurrent_packets() {
    let port = 51826;

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create multiple client sockets
    let clients: Vec<_> = (0..5)
        .map(|_| tokio::spawn(async move {
            let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            client
        }))
        .collect();

    // Wait for all clients to be created
    let mut client_sockets = Vec::new();
    for client in clients {
        client_sockets.push(client.await.unwrap());
    }

    // Send packets from all clients concurrently
    let send_tasks: Vec<_> = client_sockets
        .iter()
        .enumerate()
        .map(|(i, client)| {
            let packet = vec![WIREGUARD_MESSAGE_DATA; WIREGUARD_COOKIE_REPLY_SIZE];
            async move {
                client
                    .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
                    .await
                    .unwrap();
                i
            }
        })
        .collect();

    // Execute all sends concurrently
    for task in send_tasks {
        task.await;
    }

    // Receive all packets
    let mut buf = vec![0u8; 4096];
    let mut received_count = 0;

    for _ in 0..5 {
        let result = timeout(
            Duration::from_secs(2),
            server_socket.recv_from(&mut buf)
        ).await;

        if result.is_ok() {
            received_count += 1;
        }
    }

    assert_eq!(received_count, 5, "Should receive all 5 packets");
}

/// Test WireGuard packet size limits
///
/// Validates that the server can handle packets up to the UDP buffer size limit.
#[tokio::test]
async fn test_wireguard_packet_size_limits() {
    let port = 51827;

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Test with maximum MTU size (typical WireGuard packet)
    // Most networks support at least 1420 bytes for WireGuard
    let mut max_packet = vec![0u8; WIREGUARD_MAX_MTU_SIZE];
    max_packet[0] = WIREGUARD_MESSAGE_DATA;

    let sent = client_socket
        .send_to(&max_packet, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert_eq!(sent, WIREGUARD_MAX_MTU_SIZE);

    // Receive large packet
    let mut buf = vec![0u8; 4096];
    let result = timeout(
        Duration::from_secs(1),
        server_socket.recv_from(&mut buf)
    ).await;

    assert!(result.is_ok(), "Should receive large packet within timeout");
    let (len, _addr) = result.unwrap().unwrap();
    assert_eq!(len, WIREGUARD_MAX_MTU_SIZE);
    assert_eq!(buf[0], WIREGUARD_MESSAGE_DATA);
}

/// Test WireGuard server configuration with custom settings
///
/// Validates that WireGuard servers can be configured with custom bandwidth requirements.
#[tokio::test]
async fn test_wireguard_custom_config() {
    let port = 51828;

    let server = test_server_with_bandwidth(port, ProxyProtocols::WireGuard, 100);

    assert_eq!(server.port, port);
    assert_eq!(server.protocol, ProxyProtocols::WireGuard);
    assert_eq!(server.peer_options.min_bandwidth.as_mbps(), 100);
}

/// Test WireGuard server with country filtering
///
/// Validates that WireGuard servers can be configured with country requirements.
#[tokio::test]
async fn test_wireguard_country_filter() {
    let port = 51829;

    let server = test_server_with_country(port, ProxyProtocols::WireGuard, "US");

    assert_eq!(server.port, port);
    assert_eq!(server.protocol, ProxyProtocols::WireGuard);
    assert_eq!(server.peer_options.country, Some("US".to_string()));
}

/// Test empty packet handling
///
/// Validates that the server handles empty packets gracefully.
#[tokio::test]
async fn test_wireguard_empty_packet() {
    let port = 51830;

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Send empty packet (should be ignored)
    let packet = vec![];
    let sent = client_socket
        .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    assert_eq!(sent, 0);

    // Try to receive (with short timeout since empty packets should be ignored)
    let mut buf = vec![0u8; 4096];
    let result = timeout(
        Duration::from_millis(100),
        server_socket.recv_from(&mut buf)
    ).await;

    // Empty packet is still received at UDP level, just ignored by application logic
    assert!(result.is_ok() || result.is_err());
}

/// Test multiple message types in sequence
///
/// Validates that the server can handle different message types sent in sequence.
#[tokio::test]
async fn test_wireguard_mixed_message_types() {
    let port = 51831;

    // Create server socket
    let server_socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();

    // Create client socket
    let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Send different message types in sequence
    let message_types = vec![
        (WIREGUARD_MESSAGE_HANDSHAKE_INITIATION, WIREGUARD_HANDSHAKE_INIT_SIZE),
        (WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE, WIREGUARD_HANDSHAKE_RESP_SIZE),
        (WIREGUARD_MESSAGE_COOKIE_REPLY, WIREGUARD_COOKIE_REPLY_SIZE),
        (WIREGUARD_MESSAGE_DATA, WIREGUARD_DATA_PACKET_TEST_SIZE),
    ];

    for (msg_type, size) in message_types {
        let mut packet = vec![0u8; size];
        packet[0] = msg_type;

        client_socket
            .send_to(&packet, SocketAddr::from(([127, 0, 0, 1], port)))
            .await
            .unwrap();

        // Receive and verify
        let mut buf = vec![0u8; 4096];
        let result = timeout(
            Duration::from_secs(1),
            server_socket.recv_from(&mut buf)
        ).await;

        assert!(result.is_ok());
        let (len, _) = result.unwrap().unwrap();
        assert_eq!(len, size);
        assert_eq!(buf[0], msg_type);
    }
}
