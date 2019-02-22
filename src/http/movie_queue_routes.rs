#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    http::StatusCode, AsyncResponder, FutureResponse, HttpRequest, HttpResponse, Path, Query,
};
use failure::{err_msg, Error};
use futures::future::Future;
use std::path;
use subprocess::Exec;

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    FindNewEpisodeRequest, ImdbShowRequest, MoviePathRequest, MovieQueueRequest, ParseImdbRequest,
    QueueDeleteRequest,
};
use super::{form_http_response, get_auth_fut, unauthbody};
use crate::common::make_queue::movie_queue_http;
use crate::common::movie_queue::MovieQueueResult;
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

fn queue_body_resp(
    patterns: &[String],
    queue: &[MovieQueueResult],
) -> Result<HttpResponse, actix_web::error::Error> {
    let entries = movie_queue_http(queue)?;
    let body = movie_queue_body(patterns, &entries);
    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn movie_queue(
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let resp = request
        .state()
        .db
        .send(MovieQueueRequest {
            patterns: Vec::new(),
        })
        .from_err()
        .and_then(move |r| match r {
            Ok(queue) => queue_body_resp(&[], &queue),
            Err(err) => Err(err.into()),
        })
        .responder();

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

pub fn movie_queue_show(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let path = path.into_inner();
    let patterns = vec![path];

    let resp = request
        .state()
        .db
        .send(MovieQueueRequest {
            patterns: patterns.clone(),
        })
        .from_err()
        .and_then(move |res| match res {
            Ok(queue) => queue_body_resp(&patterns, &queue),
            Err(err) => Err(err.into()),
        })
        .responder();

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

pub fn movie_queue_delete(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let path = path.into_inner();

    let resp = request
        .state()
        .db
        .send(QueueDeleteRequest { path: path.clone() })
        .from_err()
        .and_then(move |res| match res {
            Ok(_) => Ok(form_http_response(path)),
            Err(err) => Err(err.into()),
        })
        .responder();

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

fn transcode_worker(
    directory: Option<String>,
    entries: &[MovieQueueResult],
) -> Result<HttpResponse, actix_web::Error> {
    let entries: Vec<Result<_, Error>> = entries
        .iter()
        .map(|entry| {
            remcom_single_file(&entry.path, &directory, false)?;
            Ok(format!("{}", entry))
        })
        .collect();
    let entries = map_result_vec(entries)?;
    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(entries.join("\n"));
    Ok(resp)
}

pub fn movie_queue_transcode(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let path = path.into_inner();
    let patterns = vec![path];

    let resp = request
        .state()
        .db
        .send(MovieQueueRequest {
            patterns: patterns.clone(),
        })
        .from_err()
        .and_then(move |res| match res {
            Ok(entries) => transcode_worker(None, &entries),
            Err(err) => Err(err.into()),
        })
        .responder();

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

pub fn movie_queue_transcode_directory(
    path: Path<(String, String)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let (directory, file) = path.into_inner();
    let patterns = vec![file];

    let resp = request
        .state()
        .db
        .send(MovieQueueRequest {
            patterns: patterns.clone(),
        })
        .from_err()
        .and_then(move |res| match res {
            Ok(entries) => transcode_worker(Some(directory), &entries),
            Err(err) => Err(err.into()),
        })
        .responder();

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

fn play_worker(full_path: String) -> Result<HttpResponse, actix_web::Error> {
    let path = path::Path::new(&full_path);
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let url = format!("/videos/partial/{}", file_name);

    let body = include_str!("../../templates/video_template.html").replace("VIDEO", &url);

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

pub fn movie_queue_play(
    idx: Path<i32>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let idx = idx.into_inner();

    let resp = request
        .state()
        .db
        .send(MoviePathRequest { idx })
        .from_err()
        .and_then(move |res| match res {
            Ok(full_path) => play_worker(full_path),
            Err(err) => Err(err.into()),
        })
        .responder();

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

pub fn imdb_show(
    path: Path<String>,
    query: Query<ParseImdbRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let show = path.into_inner();
    let query = query.into_inner();

    let resp = request
        .state()
        .db
        .send(ImdbShowRequest { show, query })
        .from_err()
        .and_then(move |res| match res {
            Ok(body) => Ok(form_http_response(body)),
            Err(err) => Err(err.into()),
        })
        .responder();

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

fn new_episode_worker(entries: &[String]) -> Result<HttpResponse, actix_web::Error> {
    let body =
        include_str!("../../templates/watched_template.html").replace("PREVIOUS", "/list/tvshows");
    let body = body.replace("BODY", &entries.join("\n"));
    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let resp = request
        .state()
        .db
        .send(query.into_inner())
        .from_err()
        .and_then(move |res| match res {
            Ok(entries) => new_episode_worker(&entries),
            Err(err) => Err(err.into()),
        })
        .responder();

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
