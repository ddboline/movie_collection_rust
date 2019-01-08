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
use movie_collection_rust::movie_collection::MovieCollection;
use movie_collection_rust::utils::{map_result_vec, remcom_single_file};

fn tvshows(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let shows = MovieCollection::new().print_tv_shows()?;

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
    let command = "rm -f /var/www/html/videos/partial/*";
    Exec::shell(command).join()?;

    let body = include_str!("../templates/queue_list.html");

    let queue = MovieCollection::new().print_movie_queue(&patterns)?;

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
            let entry = format!(
                "<tr>\n<td><a href={}>{}</a></td>\n<td><a href={}>imdb</a></td>",
                &format!(
                    r#""{}/{}""#,
                    "/videos/partial", file_name
                ),
                file_name,
                &format!(
                    "https://www.imdb.com/title/{}",
                    row.link.clone().unwrap_or_else(|| "".to_string())
                )
            );
            let entry = format!(
                "{}\n{}",
                entry,
                button.replace("ID", file_name).replace("SHOW", file_name)
            );

            let entry = if ext != "mp4" {
                format!(
                    r#"{}<td><button type="submit" id="{}" onclick="transcode('{}');"> transcode </button></td>"#,
                    entry, file_name, file_name)
            } else {entry};

            let command = format!("rm -f /var/www/html/videos/partial/{}", file_name);
            Exec::shell(&command).join()?;
            let command = format!(
                "ln -s {} /var/www/html/videos/partial/{}",
                row.path, file_name
            );
            Exec::shell(&command).join()?;
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

    let mq = MovieCollection::new();
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

    let mq = MovieCollection::new();
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

fn main() {
    let config = Config::with_config();
    let sys = actix::System::new("movie_queue");
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = env::var("DOMAIN").unwrap_or_else(|_| "localhost".to_string());

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
            .resource("/list", |r| r.method(Method::GET).with(movie_queue))
            .resource("/list/{show}", |r| {
                r.method(Method::GET).with(movie_queue_show)
            })
            .resource("/delete/{path}", |r| {
                r.method(Method::GET).with(movie_queue_delete)
            })
            .resource("/transcode/{path}", |r| {
                r.method(Method::GET).with(movie_queue_transcode)
            })
    })
    .bind(&format!("127.0.0.1:{}", config.port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", config.port))
    .start();

    let _ = sys.run();
}
