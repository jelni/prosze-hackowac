use std::sync::{Arc, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use async_signal::{Signal, Signals};
use base64::Engine;
use base64::prelude::BASE64_STANDARD_NO_PAD;
use clap::Parser;
use image::{ImageBuffer, ImageFormat, ImageReader, RgbImage};
use poem::endpoint::StaticFileEndpoint;
use poem::listener::TcpListener;
use poem::middleware::Tracing;
use poem::{EndpointExt, Route, Server};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;

use crate::models::{Cli, Pixel, ServerState, UpdatesBatch};

mod endpoints;
mod models;
mod network;

pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

#[expect(clippy::needless_pass_by_value, clippy::significant_drop_tightening)]
fn process_queue(
    canvas: Arc<RwLock<RgbImage>>,
    queue_rx: mpsc::Receiver<Option<Pixel>>,
    updated_pixels_tx: broadcast::Sender<String>,
) {
    while let Ok(Some(mut pixel)) = queue_rx.recv() {
        let batch_start = Instant::now();
        let mut updates_batch = UpdatesBatch::default();
        let mut canvas = canvas.write().unwrap();

        loop {
            let current_color = canvas.get_pixel_mut(pixel.x, pixel.y);
            let new_color = pixel.color();

            if *current_color != new_color {
                *current_color = new_color;
                updates_batch.add(pixel);
            }

            let Ok(new_pixel) = queue_rx
                .recv_timeout(Duration::from_millis(100).saturating_sub(batch_start.elapsed()))
            else {
                break;
            };

            let Some(new_pixel) = new_pixel else {
                return;
            };

            pixel = new_pixel;
        }

        updated_pixels_tx
            .send(BASE64_STANDARD_NO_PAD.encode(updates_batch.into_bytes()))
            .ok();
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();
    let args = Cli::parse();

    let canvas = {
        let image = ImageReader::open("data/image.png").map_or_else(
            |_| ImageBuffer::new(1920, 1080),
            |mut image_reader| {
                image_reader.set_format(ImageFormat::Png);
                image_reader.decode().unwrap().into_rgb8()
            },
        );

        Arc::new(RwLock::new(image))
    };

    let canvas_size = {
        let canvas = canvas.read().unwrap();
        (canvas.width(), canvas.height())
    };

    let (queue_tx, queue_rx) = mpsc::channel::<Option<Pixel>>();
    let (updated_pixels_tx, _) = broadcast::channel::<String>(16);
    let updated_pixels_weak = updated_pixels_tx.downgrade();

    let process_queue_handle = {
        let canvas = canvas.clone();
        thread::spawn(|| process_queue(canvas, queue_rx, updated_pixels_tx))
    };

    let tcp_listener_handle = tokio::spawn(network::tcp_listener(
        "127.0.0.1:8081",
        canvas_size,
        queue_tx.clone(),
    ));

    let udp_listener_handle = tokio::spawn(network::udp_listener(
        "127.0.0.1:8082",
        canvas_size,
        queue_tx.clone(),
    ));

    let app = Route::new()
        .at("/", StaticFileEndpoint::new("static/index.html"))
        .at("/image", poem::get(endpoints::get_image))
        .at("/updates", poem::get(endpoints::get_updates))
        .at("/pixel", poem::post(endpoints::set_pixel))
        .with(Tracing)
        .data(ServerState {
            canvas: canvas.clone(),
            canvas_size,
            queue: queue_tx.clone(),
            updated_pixels: updated_pixels_weak,
        });

    Server::new(TcpListener::bind((args.host, args.port)))
        .idle_timeout(CONNECTION_TIMEOUT)
        .run_with_graceful_shutdown(
            app,
            async {
                let mut signals = Signals::new([Signal::Term, Signal::Int]).unwrap();
                signals.next().await.unwrap().unwrap();
                queue_tx.send(None).unwrap();
            },
            Some(CONNECTION_TIMEOUT),
        )
        .await
        .unwrap();

    // tcp_listener_handle.await.unwrap();
    // udp_listener_handle.await.unwrap();
    process_queue_handle.join().unwrap();

    canvas
        .read()
        .unwrap()
        .save_with_format("data/image.png", ImageFormat::Png)
        .unwrap();
}
