use std::io::Cursor;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use async_signal::{Signal, Signals};
use futures_util::StreamExt;
use image::{ImageFormat, ImageReader, Rgb, RgbImage};
use poem::endpoint::StaticFileEndpoint;
use poem::http::StatusCode;
use poem::listener::TcpListener;
use poem::middleware::Tracing;
use poem::web::{Data, Json};
use poem::{EndpointExt, IntoResponse, Response, Route, Server, handler};
use serde::Deserialize;

#[derive(Clone)]
struct ServerState {
    canvas: Arc<RwLock<RgbImage>>,
    canvas_size: (u32, u32),
    canvas_cache: Arc<Mutex<Option<CanvasCache<Vec<u8>>>>>,
    queue: Arc<Sender<Pixel>>,
}

struct CanvasCache<T> {
    data: T,
    updated_at: Instant,
}

impl<T> CanvasCache<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            updated_at: Instant::now(),
        }
    }

    fn get(&self) -> Option<&T> {
        if self.updated_at.elapsed() >= Duration::from_millis(100) {
            return None;
        }

        Some(&self.data)
    }
}

#[derive(Deserialize)]
struct Pixel {
    x: u32,
    y: u32,
    r: u8,
    g: u8,
    b: u8,
}

#[handler]
#[expect(clippy::needless_pass_by_value)]
fn get_image(state: Data<&ServerState>) -> Response {
    if let Some(data) = state
        .canvas_cache
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|cache| cache.get())
    {
        let data = data.clone();
        return Response::from(data);
    }

    let mut buffer = Cursor::new(Vec::new());

    state
        .canvas
        .read()
        .unwrap()
        .write_to(&mut buffer, ImageFormat::Png)
        .unwrap();

    let data = buffer.into_inner();
    *state.canvas_cache.lock().unwrap() = Some(CanvasCache::new(data.clone()));

    Response::from(data)
        .set_content_type("image/png")
        .with_header("Cache-Control", "no-store")
        .into_response()
}

#[handler]
#[expect(clippy::needless_pass_by_value)]
fn set_pixel(state: Data<&ServerState>, Json(json): Json<Pixel>) -> Response {
    if json.x >= state.canvas_size.0 || json.y >= state.canvas_size.1 {
        return StatusCode::BAD_REQUEST
            .with_body("pixel outside of drawing area")
            .into_response();
    }

    state.queue.send(json).unwrap();

    StatusCode::NO_CONTENT.into()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let canvas = {
        let mut image_reader = ImageReader::open("data/image.png").unwrap();
        image_reader.set_format(ImageFormat::Png);
        let image = image_reader.decode().unwrap().into_rgb8();
        Arc::new(RwLock::new(image))
    };

    let (tx, rx) = mpsc::channel::<Pixel>();

    {
        let canvas_clone = canvas.clone();

        #[expect(clippy::significant_drop_tightening)]
        thread::spawn(move || {
            while let Ok(mut pixel) = rx.recv() {
                let mut canvas = canvas_clone.write().unwrap();

                loop {
                    canvas.put_pixel(pixel.x, pixel.y, Rgb([pixel.r, pixel.g, pixel.b]));

                    let Ok(new_pixel) = rx.try_recv() else {
                        break;
                    };

                    pixel = new_pixel;
                }
            }
        });
    }

    let app = Route::new()
        .at("/", StaticFileEndpoint::new("static/index.html"))
        .at("/image", poem::get(get_image))
        .at("/pixel", poem::post(set_pixel))
        .with(Tracing)
        .data(ServerState {
            canvas: canvas.clone(),
            canvas_size: {
                let canvas = canvas.read().unwrap();
                (canvas.width(), canvas.height())
            },
            canvas_cache: Arc::default(),
            queue: Arc::new(tx),
        });

    Server::new(TcpListener::bind("0.0.0.0:80"))
        .run_with_graceful_shutdown(
            app,
            async {
                let mut signals = Signals::new([Signal::Term, Signal::Int]).unwrap();
                signals.next().await.unwrap().unwrap();
            },
            None,
        )
        .await
        .unwrap();

    canvas
        .read()
        .unwrap()
        .save_with_format("data/image.png", ImageFormat::Png)
        .unwrap();
}
