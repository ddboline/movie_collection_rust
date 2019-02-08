pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;
pub mod trakt_routes;
pub mod tvshows_route;

use futures::Future;
use actix_web::{
    FutureResponse, HttpRequest, HttpResponse, HttpMessage, AsyncResponder
};
use movie_queue_app::AppState;


fn send_unauthorized(request: HttpRequest<AppState>) -> FutureResponse<HttpResponse> {
    request
        .body()
        .from_err()
        .and_then(move |_| Ok(HttpResponse::Unauthorized().json("Unauthorized")))
        .responder()
}
