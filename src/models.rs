use std::collections::HashMap;
use std::sync::{Arc, RwLock, mpsc};

use clap::Parser;
use image::{Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Parser)]
pub struct Cli {
    /// host to bin the server to
    #[arg(long, default_value = "localhost")]
    pub host: String,
    /// port to bind the server to
    #[arg(long, default_value_t = 80)]
    pub port: u16,
}

#[derive(Clone)]
pub struct ServerState {
    pub canvas: Arc<RwLock<RgbImage>>,
    pub canvas_size: (u32, u32),
    pub queue: mpsc::Sender<Option<Pixel>>,
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
}
