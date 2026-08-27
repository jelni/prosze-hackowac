use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use clap::Parser;
use image::{Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

#[derive(Parser)]
pub struct Cli {
    /// host to bin the server to
    #[arg(long, default_value = "localhost")]
    pub host: String,
    /// domain displayed on the page
    #[arg(long)]
    pub domain: Option<String>,
    /// port to bind the HTTP server to
    #[arg(long, default_value_t = 8080)]
    pub http_port: u16,
    /// port to bind the TCP server to
    #[arg(long, default_value_t = 8081)]
    pub tcp_port: u16,
    /// port to bind the UDP server to
    #[arg(long, default_value_t = 8082)]
    pub udp_port: u16,
}

#[derive(Clone)]
pub struct ServerState {
    pub index_html: String,
    pub canvas: Arc<RwLock<RgbImage>>,
    pub canvas_size: (u32, u32),
    pub queue: mpsc::UnboundedSender<Pixel>,
    pub updated_pixels: broadcast::WeakSender<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Pixel {
    pub x: u32,
    pub y: u32,
    r: u8,
    g: u8,
    b: u8,
}

impl Pixel {
    pub fn from_bytes(bytes: &[u8; 11]) -> Self {
        Self {
            x: u32::from_be_bytes(bytes[..4].try_into().unwrap()),
            y: u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
            r: bytes[8],
            g: bytes[9],
            b: bytes[10],
        }
    }

    pub const fn color(&self) -> Rgb<u8> {
        Rgb([self.r, self.g, self.b])
    }
}

#[derive(Default)]
pub struct UpdatesBatch {
    pixels: HashMap<(u32, u32), Rgb<u8>>,
}

impl UpdatesBatch {
    pub fn add(&mut self, pixel: Pixel) {
        self.pixels.insert((pixel.x, pixel.y), pixel.color());
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.pixels
            .into_iter()
            .flat_map(|(position, Rgb(color))| {
                <[_; _]>::from(position)
                    .into_iter()
                    .flat_map(u32::to_be_bytes)
                    .chain(color.into_iter().flat_map(u8::to_be_bytes))
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }
}
