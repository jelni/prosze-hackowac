use std::sync::mpsc;

use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, ToSocketAddrs, UdpSocket};

use crate::models::Pixel;

pub async fn tcp_listener(
    addr: impl ToSocketAddrs,
    canvas_size: (u32, u32),
    queue_tx: mpsc::Sender<Option<Pixel>>,
) {
    let listener = TcpListener::bind(addr).await.unwrap();

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();
        let queue = queue_tx.clone();

        tokio::spawn(async move {
            let mut data = [0; 11];

            loop {
                if socket.read_exact(&mut data).await.is_err() {
                    break;
                }

                let pixel = Pixel::from_bytes(&data);

                if pixel.x >= canvas_size.0 || pixel.y >= canvas_size.1 {
                    continue;
                }

                queue.send(Some(pixel)).unwrap();
            }
        });
    }
}

pub async fn udp_listener(
    addr: impl ToSocketAddrs,
    canvas_size: (u32, u32),
    queue_tx: mpsc::Sender<Option<Pixel>>,
) {
    let socket = UdpSocket::bind(addr).await.unwrap();
    // max UDP packet size excluding the header
    let mut data = [0; u16::MAX as usize - 8];

    loop {
        let Ok(len) = socket.recv(&mut data).await else {
            break;
        };

        for pixel in data[..len].as_chunks().0.into_iter().map(Pixel::from_bytes) {
            if pixel.x >= canvas_size.0 || pixel.y >= canvas_size.1 {
                continue;
            }

            queue_tx.send(Some(pixel)).unwrap();
        }
    }
}
