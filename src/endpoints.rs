use std::io::Cursor;

use image::ImageFormat;
use poem::http::StatusCode;
use poem::web::sse::{Event, SSE};
use poem::web::{Data, Json};
use poem::{IntoResponse, Response};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::CONNECTION_TIMEOUT;
use crate::models::{Pixel, ServerState};

#[poem::handler]
#[expect(clippy::needless_pass_by_value)]
pub fn index(state: Data<&ServerState>) -> Response {
    state.index_html.clone().into()
}

#[poem::handler]
#[expect(clippy::needless_pass_by_value)]
pub fn get_image(state: Data<&ServerState>) -> Response {
    let canvas = state.canvas.read().unwrap().clone();
    let mut buffer = Cursor::new(Vec::new());
    canvas.write_to(&mut buffer, ImageFormat::Png).unwrap();

    Response::from(buffer.into_inner())
        .set_content_type("image/png")
        .with_header("Cache-Control", "no-store")
        .into_response()
}

#[poem::handler]
#[expect(clippy::needless_pass_by_value)]
pub fn get_updates(state: Data<&ServerState>) -> SSE {
    let receiver = state.updated_pixels.upgrade().unwrap().subscribe();
    let stream = BroadcastStream::new(receiver)
        .map_while(|message| message.map_or(None, |message| Some(Event::message(message))));

    SSE::new(stream).keep_alive(CONNECTION_TIMEOUT)
}

#[poem::handler]
#[expect(clippy::needless_pass_by_value)]
pub fn set_pixel(state: Data<&ServerState>, Json(json): Json<Pixel>) -> Response {
    if json.x >= state.canvas_size.0 || json.y >= state.canvas_size.1 {
        return StatusCode::BAD_REQUEST
            .with_body("pixel outside of drawing area")
            .into_response();
    }

    match state.queue.send(json) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
    .into()
}
