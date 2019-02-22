pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;
pub mod trakt_routes;
pub mod tvshows_route;

use actix_web::{AsyncResponder, FutureResponse, HttpRequest, HttpResponse};
use futures::{lazy, Future};
use logged_user::LoggedUser;
use movie_queue_app::AppState;

fn get_auth_fut(
    user: &LoggedUser,
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

fn unauthbody() -> FutureResponse<HttpResponse> {
    lazy(|| Ok(HttpResponse::Unauthorized().json("Unauthorized"))).responder()
}

fn authenticated_response(
    user: &LoggedUser,
    request: &HttpRequest<AppState>,
    resp: FutureResponse<HttpResponse>,
) -> FutureResponse<HttpResponse> {
    if request.state().user_list.is_authorized(&user) {
        resp
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp,
                _ => unauthbody(),
            })
            .responder()
    }
}
