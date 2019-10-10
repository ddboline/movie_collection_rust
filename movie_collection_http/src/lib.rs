pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;
pub mod trakt_routes;
pub mod tvshows_route;

use actix::{Handler, Message};
use actix_web::web::Data;
use actix_web::HttpResponse;
use failure::Error;
use futures::Future;
use movie_collection_lib::common::pgpool::PgPool;
use movie_queue_app::AppState;
use serde::Serialize;

fn form_http_response(body: String) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body)
}

fn to_json<T>(js: &T) -> Result<HttpResponse, Error>
where
    T: Serialize,
{
    Ok(HttpResponse::Ok().json2(js))
}

fn generic_route<T, U, V>(
    query: T,
    state: Data<AppState>,
    callback: V,
) -> impl Future<Item = HttpResponse, Error = Error>
where
    T: Message<Result = Result<U, failure::Error>> + Send + 'static,
    PgPool: Handler<T, Result = Result<U, failure::Error>>,
    U: Send + 'static,
    V: FnOnce(U) -> Result<HttpResponse, Error>,
{
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(|x| callback(x)))
}

fn json_route<T, U>(
    query: T,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error>
where
    T: Message<Result = Result<U, failure::Error>> + Send + 'static,
    PgPool: Handler<T, Result = Result<U, failure::Error>>,
    U: Serialize + Send + 'static,
{
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(|x| to_json(&x)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
