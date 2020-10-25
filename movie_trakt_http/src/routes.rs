#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Path, Query},
    HttpResponse,
};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use movie_collection_lib::{
    imdb_ratings::ImdbRatings,
    movie_collection::MovieCollection,
    trakt_utils::{
        get_watchlist_shows_db_map, trakt_cal_http_worker, trakt_watched_seasons_worker,
        watch_list_http_worker, watched_action_http_worker, watchlist_action_worker,
        watchlist_worker, TRAKT_CONN,
    },
};

use super::{
    app::AppState, errors::ServiceError as Error, logged_user::LoggedUser,
    requests::WatchlistActionRequest,
};

pub type HttpResult = Result<HttpResponse, Error>;

fn form_http_response(body: String) -> HttpResult {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

pub async fn trakt_auth_url(_: LoggedUser) -> HttpResult {
    TRAKT_CONN.init().await;
    let url = TRAKT_CONN.get_auth_url().await?;
    form_http_response(url.to_string())
}

#[derive(Serialize, Deserialize)]
pub struct TraktCallbackRequest {
    pub code: StackString,
    pub state: StackString,
}

pub async fn trakt_callback(query: Query<TraktCallbackRequest>, _: LoggedUser) -> HttpResult {
    TRAKT_CONN.init().await;
    TRAKT_CONN
        .exchange_code_for_auth_token(query.code.as_str(), query.state.as_str())
        .await?;
    let body = r#"
        <title>Trakt auth code received!</title>
        This window can be closed.
        <script language="JavaScript" type="text/javascript">window.close()</script>"#;
    form_http_response(body.to_string())
}

pub async fn refresh_auth(_: LoggedUser) -> HttpResult {
    TRAKT_CONN.init().await;
    TRAKT_CONN.exchange_refresh_token().await?;
    form_http_response("finished".to_string())
}

pub async fn trakt_cal(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let entries = trakt_cal_http_worker(&state.db).await?;
    form_http_response(trakt_cal_worker(&entries).into())
}

fn trakt_cal_worker(entries: &[StackString]) -> StackString {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("")
    )
    .into()
}

pub async fn trakt_watchlist(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let shows = get_watchlist_shows_db_map(&state.db).await?;
    let entries = watchlist_worker(shows);
    form_http_response(entries)
}

pub async fn trakt_watchlist_action(
    path: Path<(StackString, StackString)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (action, imdb_url) = path.into_inner();
    let action = action.parse().expect("impossible");

    let req = WatchlistActionRequest { action, imdb_url };
    let imdb_url = req.handle(&state.db).await?;
    let body = watchlist_action_worker(action, &imdb_url).await?;
    form_http_response(body)
}

pub async fn trakt_watched_seasons(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let imdb_url = path.into_inner();
    let show_opt = ImdbRatings::get_show_by_link(&imdb_url, &state.db)
        .await?
        .map(|sh| (imdb_url, sh));
    let empty = || ("".into(), "".into(), "".into());
    let (imdb_url, show, link) =
        show_opt.map_or_else(empty, |(imdb_url, t)| (imdb_url, t.show, t.link));

    let entries = if &show == "" {
        Vec::new()
    } else {
        MovieCollection::with_pool(&state.db)?
            .print_imdb_all_seasons(&show)
            .await?
    };

    let entries = trakt_watched_seasons_worker(&link, &imdb_url, &entries)?;
    form_http_response(entries.into())
}

pub async fn trakt_watched_list(
    path: Path<(StackString, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (imdb_url, season) = path.into_inner();

    let body = watch_list_http_worker(&state.db, &imdb_url, season).await?;
    form_http_response(body.into())
}

pub async fn trakt_watched_action(
    path: Path<(StackString, StackString, i32, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (action, imdb_url, season, episode) = path.into_inner();
    let body = watched_action_http_worker(
        &state.db,
        action.parse().expect("impossible"),
        &imdb_url,
        season,
        episode,
    )
    .await?;
    form_http_response(body.into())
}
