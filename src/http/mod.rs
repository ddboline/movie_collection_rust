pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;
pub mod trakt_routes;
pub mod tvshows_route;

use actix_web::{http::StatusCode, AsyncResponder, FutureResponse, HttpRequest, HttpResponse};
use futures::{lazy, Future};
use logged_user::LoggedUser;
use movie_queue_app::AppState;
use serde::Serialize;

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

fn authenticated_response<T: 'static>(
    user: &LoggedUser,
    request: HttpRequest<AppState>,
    resp: T,
) -> FutureResponse<HttpResponse>
where
    T: FnOnce(HttpRequest<AppState>) -> FutureResponse<HttpResponse>,
{
    if request.state().user_list.is_authorized(&user) {
        resp(request)
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp(request),
                _ => unauthbody(),
            })
            .responder()
    }
}

fn to_json<T>(req: &HttpRequest<AppState>, js: &T) -> Result<HttpResponse, actix_web::Error>
where
    T: Serialize,
{
    let body = serde_json::to_string(&js)?;
    Ok(req
        .build_response(StatusCode::OK)
        .content_type("application/json")
        .body(body))
}
