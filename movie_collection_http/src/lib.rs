#[macro_use]
extern crate serde_derive;

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
use futures::Future;
use logged_user::LoggedUser;
use movie_collection_lib::common::pgpool::PgPool;
use movie_queue_app::AppState;
use serde::Serialize;

fn form_http_response(body: String) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body)
}

fn to_json<T>(js: &T) -> Result<HttpResponse, actix_web::Error>
where
    T: Serialize,
{
    Ok(HttpResponse::Ok().json2(js))
}

fn generic_route<T, U, V>(
    query: T,
    user: LoggedUser,
    state: Data<AppState>,
    callback: V,
) -> impl Future<Item = HttpResponse, Error = actix_web::Error>
where
    T: Message<Result = Result<U, failure::Error>> + Send + 'static,
    PgPool: Handler<T, Result = Result<U, failure::Error>>,
    U: Send + 'static,
    V: FnOnce(U) -> Result<HttpResponse, actix_web::Error> + 'static,
{
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| match res {
            Ok(x) => {
                if !state.user_list.is_authorized(&user) {
                    return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
                }
                callback(x)
            }
            Err(err) => Err(err.into()),
        })
}

fn json_route<T, U>(
    query: T,
    user: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = actix_web::Error>
where
    T: Message<Result = Result<U, failure::Error>> + Send + 'static,
    PgPool: Handler<T, Result = Result<U, failure::Error>>,
    U: Serialize + Send + 'static,
{
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| match res {
            Ok(x) => {
                if !state.user_list.is_authorized(&user) {
                    return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
                }
                to_json(&x)
            }
            Err(err) => Err(err.into()),
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}