use std::borrow::Cow;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_signal::{Signal, Signals};
use base64::Engine;
use base64::prelude::BASE64_STANDARD_NO_PAD;
use clap::Parser;
use image::{ImageBuffer, ImageFormat, ImageReader, RgbImage};
use poem::listener::TcpListener;
use poem::middleware::Tracing;
use poem::{EndpointExt, Route, Server};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::{self, Instant};
use tokio_stream::StreamExt;

use crate::models::{Cli, Pixel, ServerState, UpdatesBatch};

mod endpoints;
mod models;
mod network;

pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

fn get_index_html(args: &Cli, canvas_size: (u32, u32)) -> String {
    let domain = args.domain.as_ref().unwrap_or(&args.host);
    let mut template = include_str!("../static/index.html").to_owned();

    for (tag, value) in [
        (
            "http-domain",
            if args.http_port == 80 {
                Cow::Borrowed(domain)
            } else {
                Cow::Owned(format!("{domain}:{}", args.http_port))
            },
        ),
        (
            "tcp-domain",
            Cow::Owned(format!("{domain}:{}", args.tcp_port)),
        ),
        (
            "udp-domain",
            Cow::Owned(format!("{domain}:{}", args.udp_port)),
        ),
        ("width", Cow::Owned(canvas_size.0.to_string())),
        ("height", Cow::Owned(canvas_size.1.to_string())),
    ] {
        template = template.replacen(&format!("{{{{{tag}}}}}"), &value, 1);
    }

    template
}

#[expect(clippy::significant_drop_tightening)]
async fn process_queue(
    canvas: Arc<RwLock<RgbImage>>,
    mut stop_rx: watch::Receiver<()>,
    mut queue_rx: mpsc::UnboundedReceiver<Pixel>,
    updated_pixels_tx: broadcast::Sender<String>,
) {
    while let Some(mut pixel) = tokio::select! {
        biased;
        pixel = queue_rx.recv() => pixel,
        _ = stop_rx.changed() => None,
    } {
        let batch_end = Instant::now() + Duration::from_millis(100);
        let mut updates_batch = UpdatesBatch::default();

        loop {
            {
                let mut canvas = canvas.write().unwrap();

                loop {
                    let current_color = canvas.get_pixel_mut(pixel.x, pixel.y);
                    let new_color = pixel.color();

                    if *current_color != new_color {
                        *current_color = new_color;
                        updates_batch.add(pixel);
                    }

                    let Ok(new_pixel) = queue_rx.try_recv() else {
                        break;
                    };

                    pixel = new_pixel;
                }
            }

            let Some(new_pixel) = (tokio::select! {
                biased;
                pixel = queue_rx.recv() => pixel,
                () = time::sleep_until(batch_end) => None,
                _ = stop_rx.changed() => return,
            }) else {
                break;
            };

            pixel = new_pixel;
        }

        if !updates_batch.is_empty() {
            updated_pixels_tx
                .send(BASE64_STANDARD_NO_PAD.encode(updates_batch.into_bytes()))
                .ok();
        }
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

    let (stop_tx, stop_rx) = watch::channel(());
    let (queue_tx, queue_rx) = mpsc::unbounded_channel::<Pixel>();
    let (updated_pixels_tx, _) = broadcast::channel::<String>(16);
    let updated_pixels_weak = updated_pixels_tx.downgrade();

    let process_queue_handle = {
        let canvas = canvas.clone();
        let stop_rx = stop_rx.clone();
        tokio::spawn(process_queue(canvas, stop_rx, queue_rx, updated_pixels_tx))
    };

    let tcp_listener_handle = {
        let host = args.host.clone();

        tokio::spawn(network::tcp_listener(
            (host, args.tcp_port),
            canvas_size,
            stop_rx.clone(),
            queue_tx.clone(),
        ))
    };

    let udp_listener_handle = {
        let host = args.host.clone();

        tokio::spawn(network::udp_listener(
            (host, args.udp_port),
            canvas_size,
            stop_rx.clone(),
            queue_tx.clone(),
        ))
    };

    let app = Route::new()
        .at("/", poem::get(endpoints::index))
        .at("/image", poem::get(endpoints::get_image))
        .at("/updates", poem::get(endpoints::get_updates))
        .at("/pixel", poem::post(endpoints::set_pixel))
        .with(Tracing)
        .data(ServerState {
            index_html: get_index_html(&args, canvas_size),
            canvas: canvas.clone(),
            canvas_size,
            queue: queue_tx.clone(),
            updated_pixels: updated_pixels_weak,
        });

    Server::new(TcpListener::bind((args.host, args.http_port)))
        .idle_timeout(CONNECTION_TIMEOUT)
        .run_with_graceful_shutdown(
            app,
            async {
                let mut signals = Signals::new([Signal::Term, Signal::Int]).unwrap();
                signals.next().await.unwrap().unwrap();
                stop_tx.send(()).unwrap();
            },
            Some(CONNECTION_TIMEOUT),
        )
        .await
        .unwrap();

    tcp_listener_handle.await.unwrap();
    udp_listener_handle.await.unwrap();
    process_queue_handle.await.unwrap();

    canvas
        .read()
        .unwrap()
        .save_with_format("data/image.png", ImageFormat::Png)
        .unwrap();
}
