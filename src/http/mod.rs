pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;
pub mod trakt_routes;
pub mod tvshows_route;

use actix_web::{HttpRequest, HttpResponse};
use futures::Future;
use movie_queue_app::AppState;

fn unauthbody() -> HttpResponse {
    HttpResponse::Unauthorized().json("Unauthorized")
}

fn get_auth_fut(
    user: &logged_user::LoggedUser,
    request: &HttpRequest<AppState>,
) -> impl Future<Item = Result<bool, failure::Error>, Error = actix_web::Error> {
    request
        .state()
        .db
        .send(movie_queue_requests::AuthorizedUserRequest { user: user.clone() })
        .from_err()
}

fn form_http_response(body: String) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body)
}
