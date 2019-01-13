extern crate actix;
extern crate actix_web;
extern crate movie_collection_rust;
extern crate rayon;
extern crate rust_auth_server;
extern crate subprocess;

use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{http::Method, http::StatusCode, server, App, HttpResponse, Json, Path, Query};
use chrono::Duration;
use failure::Error;
use rayon::prelude::*;
use rust_auth_server::auth_handler::LoggedUser;
use std::env;
use std::path;
use subprocess::Exec;

use movie_collection_rust::config::Config;
use movie_collection_rust::movie_collection::MovieCollectionDB;
use movie_collection_rust::utils::{map_result_vec, parse_file_stem, remcom_single_file};

fn tvshows(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let shows = MovieCollectionDB::new().print_tv_shows()?;

    let body = r#"<!DOCTYPE HTML><html><body><center><H3><table border="0">BODY"#;

    let shows: Vec<_> = shows
        .into_iter()
        .map(|s| {
            format!(
                r#"<tr><td><a href="/list/{}">{}</a></td>
                   <td><a href="https://www.imdb.com/title/{}">imdb</a></tr>"#,
                s.show, s.title, s.link
            )
        })
        .collect();
    let body = body.replace("BODY", &shows.join("\n"));
    let body = format!("{}{}", body, "</table></H3></center></body></html>");

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn movie_queue_base(patterns: &[String]) -> Result<String, Error> {
    let body = include_str!("../templates/queue_list.html");

    let mq = MovieCollectionDB::new();
    let queue = mq.print_movie_queue(&patterns)?;

    let button = r#"<td><button type="submit" id="ID" onclick="delete_show('SHOW');"> remove </button></td>"#;

    let entries: Vec<Result<_, Error>> = queue
        .par_iter()
        .map(|row| {
            let path = path::Path::new(&row.path);
            let ext = path.extension().unwrap().to_str().unwrap();
            let file_name = path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            let file_stem = path.file_stem().unwrap().to_str().unwrap();
            let (show, season, episode) = parse_file_stem(&file_stem);

            let entry = if ext == "mp4" {
                let collection_idx = mq.get_collection_index(&row.path).unwrap_or(-1);
                format!(
                    "<a href={}>{}</a>",
                    &format!(
                        r#""{}/{}""#,
                        "/list/play", collection_idx
                    ), file_name
                )
            } else {
                file_name.to_string()
            };
            let entry = format!("<tr>\n<td>{}</td>\n<td><a href={}>imdb</a></td>",
                entry, &format!(
                    "https://www.imdb.com/title/{}",
                    row.link.clone().unwrap_or_else(|| "".to_string())));
            let entry = format!(
                "{}\n{}",
                entry,
                button.replace("ID", file_name).replace("SHOW", file_name)
            );

            let entry = if ext != "mp4" {
                if season != -1 && episode != -1 {
                    format!(
                        r#"{}<td><button type="submit" id="{}" onclick="transcode('{}');"> transcode </button></td>"#,
                        entry, file_name, file_name)
                } else {
                    let entries: Vec<_> = row.path.split("/").collect();
                    let len_entries = entries.len();
                    let directory = entries.iter().nth(len_entries-2).unwrap();
                    format!(
                        r#"{}<td><button type="submit" id="{}" onclick="transcode_directory('{}', '{}');"> transcode </button></td>"#,
                        entry, file_name, file_name, directory)
                }
            } else {entry};

            Ok(entry)
        })
        .collect();

    let entries = map_result_vec(entries)?;

    let body = body.replace("BODY", &entries.join("\n"));
    Ok(body)
}

fn movie_queue(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let body = movie_queue_base(&[])?;

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

    let body = movie_queue_base(&[path])?;

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

fn movie_queue_delete(path: Path<String>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let mq = MovieCollectionDB::new();
    println!("{}", path);
    let index = mq.get_collection_index_match(&path.into_inner())?;
    mq.remove_from_queue_by_collection_idx(index)?;

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(index.to_string());
    Ok(resp)
}

fn movie_queue_transcode(path: Path<String>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let mq = MovieCollectionDB::new();
    println!("{}", path);
    for entry in mq.print_movie_queue(&[path.into_inner()])? {
        println!("{}", entry);
        remcom_single_file(&entry.path, &None, false);
    }

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body("".to_string());
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

    let mq = MovieCollectionDB::new();
    println!("{}", file);
    for entry in mq.print_movie_queue(&[file])? {
        println!("{}", entry);
        remcom_single_file(&entry.path, &Some(directory.clone()), false);
    }

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body("".to_string());
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
            .resource("/tvshows", |r| r.method(Method::GET).with(tvshows))
            .resource("/list/{show}", |r| {
                r.method(Method::GET).with(movie_queue_show)
            })
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
            .resource("/list", |r| r.method(Method::GET).with(movie_queue))
    })
    .bind(&format!("127.0.0.1:{}", port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", port))
    .start();

    let _ = sys.run();
}
