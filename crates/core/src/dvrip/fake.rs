use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::client::Connection;
use super::frame::Header;

pub(crate) async fn serve(replies: Vec<Vec<u8>>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut seen = Vec::new();
        for reply in replies {
            let mut header_bytes = [0u8; super::HEADER_LEN];
            if socket.read_exact(&mut header_bytes).await.is_err() {
                break;
            }
            let header = Header::decode(&header_bytes).unwrap();
            let mut body = vec![0u8; header.length as usize];
            socket.read_exact(&mut body).await.unwrap();
            seen.push(String::from_utf8_lossy(&body).into_owned());

            let response = Header {
                session: 0x1122_3344,
                sequence: header.sequence,
                channel: 0,
                end: true,
                id: header.id.wrapping_add(1),
                length: reply.len() as u32,
            };
            socket.write_all(&response.encode()).await.unwrap();
            socket.write_all(&reply).await.unwrap();
        }
        seen
    });
    (format!("{addr}"), handle)
}

pub(crate) fn json_reply(value: Value) -> Vec<u8> {
    let mut payload = value.to_string().into_bytes();
    payload.push(b'\n');
    payload.push(0);
    payload
}

pub(crate) async fn connect(address: &str) -> Connection {
    Connection::connect(address).await.unwrap()
}

pub(crate) async fn serve_scripted<F>(
    mut handler: F,
) -> (String, tokio::task::JoinHandle<Vec<(u8, u16, Vec<u8>)>>)
where
    F: FnMut(u8, u16, &[u8]) -> Vec<(u16, Vec<u8>)> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut seen: Vec<(u8, u16, Vec<u8>)> = Vec::new();
        loop {
            let mut header_bytes = [0u8; super::HEADER_LEN];
            if socket.read_exact(&mut header_bytes).await.is_err() {
                break;
            }
            let header = Header::decode(&header_bytes).unwrap();
            let mut body = vec![0u8; header.length as usize];
            if socket.read_exact(&mut body).await.is_err() {
                break;
            }
            seen.push((header.channel, header.id, body.clone()));
            for (id, payload) in handler(header.channel, header.id, &body) {
                let response = Header {
                    session: header.session,
                    sequence: header.sequence,
                    channel: header.channel,
                    end: false,
                    id,
                    length: payload.len() as u32,
                };
                if socket.write_all(&response.encode()).await.is_err() {
                    return seen;
                }
                if socket.write_all(&payload).await.is_err() {
                    return seen;
                }
            }
        }
        seen
    });
    (format!("{addr}"), handle)
}
