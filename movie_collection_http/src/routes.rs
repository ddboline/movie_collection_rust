#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Path, Query},
    HttpResponse,
};
use anyhow::format_err;
use maplit::hashmap;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{os::unix::fs::symlink, path};

use movie_collection_lib::{
    movie_collection::{find_new_episodes_http_worker, MovieCollection},
    movie_queue::{MovieQueueDB},
    trakt_utils::{get_watchlist_shows_db_map, tvshows_worker},
    tv_show_source::TvShowSource,
    utils::HBR,
};

use super::{
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    app::AppState,
    requests::{
        ImdbRatingsSetSourceRequest, ImdbShowRequest,
        ParseImdbRequest,
    },
};

pub type HttpResult = Result<HttpResponse, Error>;

fn form_http_response(body: String) -> HttpResult {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn to_json(js: impl Serialize) -> HttpResult {
    Ok(HttpResponse::Ok().json(js))
}

pub async fn frontpage(_: LoggedUser) -> HttpResult {
    form_http_response(HBR.render("index.html", &hashmap! {"BODY" => ""})?)
}

#[derive(Serialize, Deserialize)]
pub struct FindNewEpisodeRequest {
    pub source: Option<TvShowSource>,
    pub shows: Option<StackString>,
}

pub async fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let entries = find_new_episodes_http_worker(&state.db, req.shows, req.source).await?;
    form_http_response(new_episode_worker(&entries).into())
}

fn new_episode_worker(entries: &[StackString]) -> StackString {
    let previous = r#"
        <a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>
        <input type="button" name="list_cal" value="TVCalendar" onclick="updateMainArticle('/list/cal');"/>
        <input type="button" name="list_cal" value="NetflixCalendar" onclick="updateMainArticle('/list/cal?source=netflix');"/>
        <input type="button" name="list_cal" value="AmazonCalendar" onclick="updateMainArticle('/list/cal?source=amazon');"/>
        <input type="button" name="list_cal" value="HuluCalendar" onclick="updateMainArticle('/list/cal?source=hulu');"/><br>
        <button name="remcomout" id="remcomoutput"> &nbsp; </button>
    "#;
    format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("")
    )
    .into()
}

pub async fn tvshows(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let shows = MovieCollection::with_pool(&state.db)?
        .print_tv_shows()
        .await?;
    let tvshowsmap = get_watchlist_shows_db_map(&state.db).await?;
    let entries = tvshows_worker(tvshowsmap, shows)?;
    form_http_response(entries.into())
}

pub async fn user(user: LoggedUser) -> HttpResult {
    to_json(user)
}

pub async fn movie_queue_delete(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let path = path.into_inner();
    if std::path::Path::new(path.as_str()).exists() {
        MovieQueueDB::with_pool(&state.db)
            .remove_from_queue_by_path(&path)
            .await?;
    }
    form_http_response(path.into())
}

pub async fn movie_queue_play(idx: Path<i32>, _: LoggedUser, state: Data<AppState>) -> HttpResult {
    let idx = idx.into_inner();

    let movie_path = MovieCollection::with_pool(&state.db)?
        .get_collection_path(idx)
        .await?;
    let movie_path = std::path::Path::new(movie_path.as_str());
    let body = play_worker(&movie_path)?;
    form_http_response(body)
}

fn play_worker(full_path: &path::Path) -> Result<String, Error> {
    let file_name = full_path
        .file_name()
        .ok_or_else(|| format_err!("Invalid path"))?
        .to_string_lossy();
    let url = format!("/videos/partial/{}", file_name);

    let body = format!(
        r#"{}<br>
            <video width="720" controls>
            <source src="{}" type="video/mp4">
            Your browser does not support HTML5 video.
            </video>
        "#,
        file_name, url
    );

    let partial_path =
        std::path::Path::new("/var/www/html/videos/partial").join(file_name.as_ref());
    if partial_path.exists() {
        std::fs::remove_file(&partial_path)?;
    }

    symlink(&full_path, &partial_path)?;
    Ok(body)
}

pub async fn imdb_show(
    path: Path<StackString>,
    query: Query<ParseImdbRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let show = path.into_inner();
    let query = query.into_inner();

    let req = ImdbShowRequest { show, query };
    let body = req.handle(&state.db).await?;
    form_http_response(body.into())
}

pub async fn imdb_ratings_set_source(
    query: Query<ImdbRatingsSetSourceRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let query = query.into_inner();
    query.handle(&state.db).await?;
    form_http_response("Success".to_string())
}

