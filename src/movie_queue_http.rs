#![allow(clippy::needless_pass_by_value)]

extern crate actix;
extern crate actix_web;
extern crate movie_collection_rust;
extern crate rust_auth_server;
extern crate subprocess;

use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{http::Method, http::StatusCode, server, App, HttpResponse, Path};
use chrono::Duration;
use failure::Error;
use rust_auth_server::auth_handler::LoggedUser;
use std::collections::{HashMap, HashSet};
use std::path;
use subprocess::Exec;

use movie_collection_rust::common::config::Config;
use movie_collection_rust::common::imdb_ratings::ImdbRatings;
use movie_collection_rust::common::make_queue::movie_queue_http;
use movie_collection_rust::common::movie_collection::{MovieCollection, MovieCollectionDB};
use movie_collection_rust::common::movie_queue::MovieQueueDB;
use movie_collection_rust::common::parse_imdb::parse_imdb_worker;
use movie_collection_rust::common::pgpool::PgPool;
use movie_collection_rust::common::trakt_utils::{
    get_watched_shows_db, get_watchlist_shows_db_map, TraktConnection, WatchListShow,
    WatchedEpisode, WatchedMovie,
};
use movie_collection_rust::common::utils::{map_result_vec, remcom_single_file};

fn tvshows(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let mc = MovieCollectionDB::new();

    let tvshows: HashMap<String, _> = mc
        .print_tv_shows()?
        .into_iter()
        .map(|s| (s.link.clone(), (s.show, s.title, s.link, s.source)))
        .collect();
    let watchlist: HashMap<String, _> = get_watchlist_shows_db_map(&mc.pool)?
        .into_iter()
        .map(|(link, (show, s, source))| (link, (show, s.title, s.link, source)))
        .collect();
    let watchlist_shows: Vec<_> = watchlist
        .iter()
        .filter_map(|(_, (show, title, link, source))| match tvshows.get(link) {
            None => Some((show.clone(), title.clone(), link.clone(), source.clone())),
            Some(_) => None,
        })
        .collect();

    let mut shows: Vec<_> = tvshows
        .iter()
        .map(|(_, v)| v)
        .chain(watchlist_shows.iter())
        .collect();
    shows.sort_by_key(|(s, _, _, _)| s);

    let body = include_str!("../templates/tvshows_template.html");

    let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
    let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

    let shows: Vec<_> = shows
        .into_iter()
        .map(|(show, title, link, source)| {
            let has_watchlist = watchlist.contains_key(link);
            format!(
                r#"<tr><td>{}</td>
                   <td><a href="https://www.imdb.com/title/{}">imdb</a></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                if tvshows.contains_key(link) {
                    format!(r#"<a href="/list/{}">{}</a>"#, show, title)
                } else {
                    format!(
                        r#"<a href="/list/trakt/watched/list/{}">{}</a>"#,
                        link, title
                    )
                },
                link,
                match source.as_ref().map(|s| s.as_str()) {
                    Some("netflix") => r#"<a href="https://netflix.com">netflix</a>"#,
                    Some("hulu") => r#"<a href="https://hulu.com">netflix</a>"#,
                    Some("amazon") => r#"<a href="https://amazon.com">netflix</a>"#,
                    _ => "",
                },
                if has_watchlist {
                    format!(r#"<a href="/list/trakt/watched/list/{}">watchlist</a>"#, link)
                } else {
                    "".to_string()
                },
                if !has_watchlist {
                    button_add.replace("SHOW", link)
                } else {
                    button_rm.replace("SHOW", link)
                },
            )
        })
        .collect();

    let body = body.replace("BODY", &shows.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn movie_queue(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let body = movie_queue_http(&[])?;

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn movie_queue_show(path: Path<String>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let path = path.into_inner();

    let body = movie_queue_http(&[&path])?;

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn movie_queue_delete(path: Path<String>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let config = Config::with_config();
    let pool = PgPool::new(&config.pgurl);
    let mq = MovieQueueDB::with_pool(pool);
    let path = path.into_inner();

    if path::Path::new(&path).exists() {
        mq.remove_from_queue_by_path(&path)?;
    }

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(path);
    Ok(resp)
}

fn movie_queue_transcode(path: Path<String>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let path = path.into_inner();

    let mq = MovieQueueDB::new();

    let entries: Vec<Result<_, Error>> = mq
        .print_movie_queue(&[&path])?
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

fn movie_queue_transcode_directory(
    path: Path<(String, String)>,
    user: LoggedUser,
) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }
    let (directory, file) = path.into_inner();

    let mc = MovieQueueDB::new();

    let entries: Vec<Result<_, Error>> = mc
        .print_movie_queue(&[&file])?
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

fn movie_queue_play(idx: Path<i32>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let mq = MovieCollectionDB::new();
    let full_path = mq.get_collection_path(idx.into_inner())?;

    let path = path::Path::new(&full_path);
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let url = format!("/videos/partial/{}", file_name);

    let body = include_str!("../templates/video_template.html").replace("VIDEO", &url);

    let command = format!("rm -f /var/www/html/videos/partial/{}", file_name);
    Exec::shell(&command).join()?;
    let command = format!(
        "ln -s {} /var/www/html/videos/partial/{}",
        full_path, file_name
    );
    Exec::shell(&command).join()?;

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn trakt_cal(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }
    let body = "";
    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn trakt_watchlist(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }
    let mc = MovieCollectionDB::new();
    let mut shows: Vec<_> = get_watchlist_shows_db_map(&mc.pool)?
        .into_iter()
        .map(|(_, (_, s, source))| (s.title, s.link, source))
        .collect();

    shows.sort();

    let body =
        include_str!("../templates/watchlist_template.html").replace("PREVIOUS", "/list/tvshows");

    let shows: Vec<_> = shows
        .into_iter()
        .map(|(title, link, source)| {
            format!(
                r#"<tr><td>{}</td>
                   <td><a href="https://www.imdb.com/title/{}">imdb</a> {} </tr>"#,
                format!(
                    r#"<a href="/list/trakt/watched/list/{}">{}</a>"#,
                    link, title
                ),
                link,
                match source.as_ref().map(|s| s.as_str()) {
                    Some("netflix") => r#"<td><a href="https://netflix.com">netflix</a>"#,
                    Some("hulu") => r#"<td><a href="https://hulu.com">netflix</a>"#,
                    Some("amazon") => r#"<td><a href="https://amazon.com">netflix</a>"#,
                    _ => "",
                },
            )
        })
        .collect();

    let body = body.replace("BODY", &shows.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn trakt_watchlist_action(
    path: Path<(String, String)>,
    user: LoggedUser,
) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let (action, imdb_url) = path.into_inner();

    let ti = TraktConnection::new();
    let mc = MovieCollectionDB::new();

    let body = match action.as_str() {
        "add" => {
            let result = ti.add_watchlist_show(&imdb_url)?;
            if let Some(show) = ti.get_watchlist_shows()?.get(&imdb_url) {
                show.insert_show(&mc.pool)?;
            }
            format!("{}", result)
        }
        "rm" => {
            let result = ti.remove_watchlist_show(&imdb_url)?;
            if let Some(show) = WatchListShow::get_show_by_link(&imdb_url, &mc.pool)? {
                show.delete_show(&mc.pool)?;
            }
            format!("{}", result)
        }
        _ => "".to_string(),
    };

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn trakt_watched_seasons(path: Path<String>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let imdb_url = path.into_inner();

    let mc = MovieCollectionDB::new();

    let body = include_str!("../templates/watchlist_template.html")
        .replace("PREVIOUS", "/list/trakt/watchlist");

    let show_opt = ImdbRatings::get_show_by_link(&imdb_url, &mc.pool)?;

    let entries = if let Some(show) = show_opt {
        mc.print_imdb_all_seasons(&show.show)?
            .iter()
            .map(|s| {
                format!(
                    "<tr><td>{}<td>{}<td>{}</tr>",
                    format!(
                        r#"<a href="/list/trakt/watched/list/{}/{}">{}</t>"#,
                        imdb_url, s.season, s.title
                    ),
                    s.season,
                    s.nepisodes
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let body = body.replace("BODY", &entries.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn trakt_watched_list(path: Path<(String, i32)>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let (imdb_url, season) = path.into_inner();

    let mc = MovieCollectionDB::new();
    let mq = MovieQueueDB::with_pool(mc.pool.clone());

    let watched_shows_db: HashSet<(String, i32, i32)> = get_watched_shows_db(&mc.pool)?
        .into_iter()
        .map(|s| (s.imdb_url.clone(), s.season, s.episode))
        .collect();

    let show_opt = ImdbRatings::get_show_by_link(&imdb_url, &mc.pool)?;

    let patterns = match show_opt.as_ref() {
        Some(show) => vec![show.show.as_str()],
        None => Vec::new(),
    };

    let queue: HashMap<(String, i32, i32), _> = mq
        .print_movie_queue(&patterns)?
        .into_iter()
        .filter_map(|s| match &s.show {
            Some(show) => match s.season {
                Some(season) => match s.episode {
                    Some(episode) => Some(((show.clone(), season, episode), s)),
                    None => None,
                },
                None => None,
            },
            None => None,
        })
        .collect();

    let button_add = r#"<td><button type="submit" id="ID" onclick="watched_add('SHOW', SEASON, EPISODE);">add to watched</button></td>"#;
    let button_rm = r#"<td><button type="submit" id="ID" onclick="watched_rm('SHOW', SEASON, EPISODE);">remove from watched</button></td>"#;

    let body = include_str!("../templates/watched_template.html").replace(
        "PREVIOUS",
        &format!("/list/trakt/watched/list/{}", imdb_url),
    );

    let entries = if let Some(show) = show_opt {
        mc.print_imdb_episodes(&show.show, Some(season))?
            .iter()
            .map(|s| {
                let entry = if let Some(row) = queue.get(&(show.show.clone(), season, s.episode)) {
                    let collection_idx = mc.get_collection_index(&row.path).unwrap().unwrap_or(-1);
                    format!(
                        "<a href={}>{}</a>",
                        &format!(r#""{}/{}""#, "/list/play", collection_idx),
                        s.title
                    )
                } else {
                    s.title.clone()
                };

                format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    entry,
                    season,
                    s.episode,
                    s.airdate,
                    if watched_shows_db.contains(&(imdb_url.clone(), season, s.episode)) {
                        button_rm
                            .replace("SHOW", &imdb_url)
                            .replace("SEASON", &season.to_string())
                            .replace("EPISODE", &s.episode.to_string())
                    } else {
                        button_add
                            .replace("SHOW", &imdb_url)
                            .replace("SEASON", &season.to_string())
                            .replace("EPISODE", &s.episode.to_string())
                    }
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let body = body.replace("BODY", &entries.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn trakt_watched_action(
    path: Path<(String, String, i32, i32)>,
    user: LoggedUser,
) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let (action, imdb_url, season, episode) = path.into_inner();

    let ti = TraktConnection::new();
    let mc = MovieCollectionDB::new();

    let body = match action.as_str() {
        "add" => {
            let result = if season != -1 && episode != -1 {
                ti.add_episode_to_watched(&imdb_url, season, episode)?
            } else {
                ti.add_movie_to_watched(&imdb_url)?
            };
            if season != -1 && episode != -1 {
                WatchedEpisode {
                    imdb_url: imdb_url.clone(),
                    season,
                    episode,
                    ..Default::default()
                }
                .insert_episode(&mc.pool)?;
            } else {
                WatchedMovie {
                    imdb_url,
                    title: "".to_string(),
                }
                .insert_movie(&mc.pool)?;
            }

            format!("{}", result)
        }
        "rm" => {
            let result = if season != -1 && episode != -1 {
                ti.remove_episode_to_watched(&imdb_url, season, episode)?
            } else {
                ti.remove_movie_to_watched(&imdb_url)?
            };

            if season != -1 && episode != -1 {
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, episode)?
                {
                    epi_.delete_episode(&mc.pool)?;
                }
            } else {
                if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url)? {
                    movie.delete_movie(&mc.pool)?;
                }
            };

            format!("{}", result)
        }
        _ => "".to_string(),
    };

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn imdb_show(path: Path<String>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let show = path.into_inner();

    let output: Vec<_> = parse_imdb_worker(&show, false, None, false, None, false, false)?
        .iter()
        .map(|line| format!("<tr><td>{}</td></tr>", line.join("</td><td>")))
        .collect();

    let body = include_str!("../templates/watched_template.html");

    let body = body.replace("BODY", &output.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn main() {
    let config = Config::with_config();
    let command = "rm -f /var/www/html/videos/partial/*";
    Exec::shell(command).join().unwrap();

    let sys = actix::System::new("movie_queue");
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = config.domain.clone();
    let port = config.port;

    server::new(move || {
        App::new()
            .middleware(IdentityService::new(
                CookieIdentityPolicy::new(secret.as_bytes())
                    .name("auth")
                    .path("/")
                    .domain(domain.as_str())
                    .max_age(Duration::days(1))
                    .secure(false), // this can only be true if you have https
            ))
            .resource("/list/tvshows", |r| r.method(Method::GET).with(tvshows))
            .resource("/list/delete/{path}", |r| {
                r.method(Method::GET).with(movie_queue_delete)
            })
            .resource("/list/transcode/{file}", |r| {
                r.method(Method::GET).with(movie_queue_transcode)
            })
            .resource("/list/transcode/{directory}/{file}", |r| {
                r.method(Method::GET).with(movie_queue_transcode_directory)
            })
            .resource("/list/play/{index}", |r| {
                r.method(Method::GET).with(movie_queue_play)
            })
            .resource("/list/trakt/cal", |r| r.method(Method::GET).with(trakt_cal))
            .resource("/list/trakt/watchlist", |r| {
                r.method(Method::GET).with(trakt_watchlist)
            })
            .resource("/list/trakt/watchlist/{action}/{imdb_url}", |r| {
                r.method(Method::GET).with(trakt_watchlist_action)
            })
            .resource("/list/trakt/watched/list/{imdb_url}", |r| {
                r.method(Method::GET).with(trakt_watched_seasons)
            })
            .resource("/list/trakt/watched/list/{imdb_url}/{season}", |r| {
                r.method(Method::GET).with(trakt_watched_list)
            })
            .resource(
                "/list/trakt/watched/{action}/{imdb_url}/{season}/{episode}",
                |r| r.method(Method::GET).with(trakt_watched_action),
            )
            .resource("/list/{show}", |r| {
                r.method(Method::GET).with(movie_queue_show)
            })
            .resource("/list/imdb/{show}", |r| {
                r.method(Method::GET).with(imdb_show)
            })
            .resource("/list", |r| r.method(Method::GET).with(movie_queue))
    })
    .bind(&format!("127.0.0.1:{}", port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", port))
    .start();

    let _ = sys.run();
}
