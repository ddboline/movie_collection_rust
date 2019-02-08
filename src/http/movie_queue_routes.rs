#![allow(clippy::needless_pass_by_value)]

extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate rust_auth_server;
extern crate subprocess;

use actix_web::{
    http::StatusCode, AsyncResponder, FutureResponse, HttpRequest, HttpResponse, Path,
    Query,
};
use failure::{err_msg, Error};
use futures::future::Future;
use rust_auth_server::auth_handler::LoggedUser;
use std::path;
use subprocess::Exec;

use super::send_unauthorized;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    ImdbShowRequest, MoviePathRequest, MovieQueueRequest,
    ParseImdbRequest, QueueDeleteRequest,
};
use crate::common::make_queue::movie_queue_http;
use crate::common::utils::{map_result_vec, remcom_single_file};

fn movie_queue_body(patterns: &[String], entries: &[String]) -> String {
    let watchlist_url = if patterns.is_empty() {
        "/list/trakt/watchlist".to_string()
    } else {
        format!("/list/trakt/watched/list/{}", patterns.join("_"))
    };

    let body = include_str!("../../templates/queue_list.html").replace("WATCHLIST", &watchlist_url);
    body.replace("BODY", &entries.join("\n"))
}

pub fn movie_queue(
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: Vec::new(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(queue) => {
                    let entries = movie_queue_http(&queue)?;
                    let body = movie_queue_body(&[], &entries);
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_show(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let path = path.into_inner();
        let patterns = vec![path];
        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: patterns.clone(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(queue) => {
                    let entries = movie_queue_http(&queue)?;
                    let body = movie_queue_body(&patterns, &entries);
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_delete(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let path = path.into_inner();

        request
            .state()
            .db
            .send(QueueDeleteRequest { path: path.clone() })
            .from_err()
            .and_then(move |res| match res {
                Ok(_) => {
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(path);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_transcode(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let path = path.into_inner();

        let patterns = vec![path];

        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: patterns.clone(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(entries) => {
                    let entries: Vec<Result<_, Error>> = entries
                        .iter()
                        .map(|entry| {
                            remcom_single_file(&entry.path, &None, false)?;
                            Ok(format!("{}", entry))
                        })
                        .collect();
                    let entries = map_result_vec(entries)?;
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(entries.join("\n"));
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_transcode_directory(
    path: Path<(String, String)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let (directory, file) = path.into_inner();

        let patterns = vec![file];

        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: patterns.clone(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(entries) => {
                    let entries: Vec<Result<_, Error>> = entries
                        .iter()
                        .map(|entry| {
                            remcom_single_file(&entry.path, &Some(directory.clone()), false)?;
                            Ok(format!("{}", entry))
                        })
                        .collect();

                    let entries = map_result_vec(entries)?;

                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(entries.join("\n"));
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_play(
    idx: Path<i32>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let idx = idx.into_inner();

        request
            .state()
            .db
            .send(MoviePathRequest { idx })
            .from_err()
            .and_then(move |res| match res {
                Ok(full_path) => {
                    let path = path::Path::new(&full_path);
                    let file_name = path.file_name().unwrap().to_str().unwrap();
                    let url = format!("/videos/partial/{}", file_name);

                    let body =
                        include_str!("../../templates/video_template.html").replace("VIDEO", &url);

                    let command = format!("rm -f /var/www/html/videos/partial/{}", file_name);
                    Exec::shell(&command).join().map_err(err_msg)?;
                    let command = format!(
                        "ln -s {} /var/www/html/videos/partial/{}",
                        full_path, file_name
                    );
                    Exec::shell(&command).join().map_err(err_msg)?;

                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}


pub fn imdb_show(
    path: Path<String>,
    query: Query<ParseImdbRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let show = path.into_inner();
        let query = query.into_inner();

        request
            .state()
            .db
            .send(ImdbShowRequest { show, query })
            .from_err()
            .and_then(move |res| match res {
                Ok(body) => {
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}
