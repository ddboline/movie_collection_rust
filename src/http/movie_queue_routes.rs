#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    http::StatusCode, FutureResponse, HttpRequest, HttpResponse, Json, Path, Query,
};
use failure::{err_msg, Error};
use std::path;
use subprocess::Exec;

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    FindNewEpisodeRequest, ImdbEpisodesSyncRequest, ImdbEpisodesUpdateRequest,
    ImdbRatingsSyncRequest, ImdbRatingsUpdateRequest, ImdbShowRequest, LastModifiedRequest,
    MovieCollectionSyncRequest, MovieCollectionUpdateRequest, MoviePathRequest, MovieQueueRequest,
    MovieQueueSyncRequest, MovieQueueUpdateRequest, ParseImdbRequest, QueueDeleteRequest,
};
use super::{form_http_response, generic_route, json_route};
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
    generic_route(
        MovieQueueRequest {
            patterns: Vec::new(),
        },
        user,
        request,
        move |(queue, _)| queue_body_resp(&[], &queue),
    )
}

pub fn movie_queue_show(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let path = path.into_inner();
    let patterns = vec![path];

    generic_route(
        MovieQueueRequest { patterns },
        user,
        request,
        move |(queue, patterns)| queue_body_resp(&patterns, &queue),
    )
}

pub fn movie_queue_delete(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let path = path.into_inner();

    generic_route(QueueDeleteRequest { path }, user, request, move |path| {
        Ok(form_http_response(path))
    })
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

    generic_route(
        MovieQueueRequest { patterns },
        user,
        request,
        move |(entries, _)| transcode_worker(None, &entries),
    )
}

pub fn movie_queue_transcode_directory(
    path: Path<(String, String)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let (directory, file) = path.into_inner();
    let patterns = vec![file];

    generic_route(
        MovieQueueRequest { patterns },
        user,
        request,
        move |(entries, _)| transcode_worker(Some(directory), &entries),
    )
}

fn play_worker(full_path: String) -> Result<HttpResponse, actix_web::Error> {
    let path = path::Path::new(&full_path);

    let file_name = path
        .file_name()
        .ok_or_else(|| err_msg("Invalid path"))?
        .to_str()
        .ok_or_else(|| err_msg("Invalid utf8"))?;
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

    generic_route(MoviePathRequest { idx }, user, request, play_worker)
}

pub fn imdb_show(
    path: Path<String>,
    query: Query<ParseImdbRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let show = path.into_inner();
    let query = query.into_inner();

    generic_route(
        ImdbShowRequest { show, query },
        user,
        request,
        move |body| Ok(form_http_response(body)),
    )
}

fn new_episode_worker(entries: Vec<String>) -> Result<HttpResponse, actix_web::Error> {
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
    generic_route(query.into_inner(), user, request, new_episode_worker)
}

pub fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    json_route(query.into_inner(), user, request)
}

pub fn imdb_episodes_update(
    data: Json<ImdbEpisodesUpdateRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let episodes = data.into_inner();

    generic_route(episodes, user, request, move |_| {
        Ok(form_http_response("Success".to_string()))
    })
}

pub fn imdb_ratings_route(
    query: Query<ImdbRatingsSyncRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    json_route(query.into_inner(), user, request)
}

pub fn imdb_ratings_update(
    data: Json<ImdbRatingsUpdateRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let shows = data.into_inner();

    generic_route(shows, user, request, move |_| {
        Ok(form_http_response("Success".to_string()))
    })
}

pub fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    json_route(query.into_inner(), user, request)
}

pub fn movie_queue_update(
    data: Json<MovieQueueUpdateRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let queue = data.into_inner();

    generic_route(queue, user, request, move |_| {
        Ok(form_http_response("Success".to_string()))
    })
}

pub fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    json_route(query.into_inner(), user, request)
}

pub fn movie_collection_update(
    data: Json<MovieCollectionUpdateRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let collection = data.into_inner();

    generic_route(collection, user, request, move |_| {
        Ok(form_http_response("Success".to_string()))
    })
}

pub fn last_modified_route(
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    json_route(LastModifiedRequest {}, user, request)
}
