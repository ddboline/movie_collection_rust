extern crate actix;
extern crate actix_web;
extern crate rayon;
extern crate rust_auth_server;
extern crate subprocess;

use failure::Error;
use rayon::prelude::*;

use std::path;

use crate::common::config::Config;
use crate::common::movie_collection::MovieCollectionDB;
use crate::common::movie_queue::MovieQueueDB;
use crate::common::pgpool::PgPool;
use crate::common::utils::{get_video_runtime, map_result_vec, parse_file_stem};

pub fn make_queue_worker(
    add_files: Option<Vec<String>>,
    del_files: Option<Vec<String>>,
    do_time: bool,
    patterns: &[&str],
    do_shows: bool,
) -> Result<(), Error> {
    let config = Config::with_config();
    let pool = PgPool::new(&config.pgurl);
    let mc = MovieCollectionDB::with_pool(pool.clone());
    let mq = MovieQueueDB::with_pool(pool);

    if do_shows {
        for show in mc.print_tv_shows()? {
            println!("{}", show);
        }
    } else if let Some(files) = del_files {
        for file in files {
            if let Ok(idx) = file.parse::<i32>() {
                mq.remove_from_queue_by_idx(idx)?;
            } else {
                mq.remove_from_queue_by_path(&file)?;
            }
        }
    } else if let Some(files) = add_files {
        if files.len() == 1 {
            let max_idx = mq.get_max_queue_index()?;
            mq.insert_into_queue(max_idx + 1, &files[0])?;
        } else if files.len() == 2 {
            if let Ok(idx) = files[0].parse::<i32>() {
                println!("inserting into {}", idx);
                mq.insert_into_queue(idx, &files[1])?;
            } else {
                for file in &files {
                    let max_idx = mq.get_max_queue_index()?;
                    mq.insert_into_queue(max_idx + 1, &file)?;
                }
            }
        } else {
            for file in &files {
                let max_idx = mq.get_max_queue_index()?;
                mq.insert_into_queue(max_idx + 1, &file)?;
            }
        }
    } else {
        let results = mq.print_movie_queue(&patterns)?;
        if do_time {
            let results: Vec<Result<_, Error>> = results
                .into_par_iter()
                .map(|result| {
                    let timeval = get_video_runtime(&result.path)?;
                    Ok((timeval, result))
                })
                .collect();

            let results = map_result_vec(results)?;

            for (timeval, result) in results {
                println!("{} {}", result, timeval);
            }
        } else {
            for result in results {
                println!("{}", result);
            }
        }
    }
    Ok(())
}

pub fn movie_queue_http(patterns: &[&str]) -> Result<String, Error> {
    let body = include_str!("../../templates/queue_list.html");

    let config = Config::with_config();
    let pool = PgPool::new(&config.pgurl);
    let mc = MovieCollectionDB::with_pool(pool.clone());
    let mq = MovieQueueDB::with_pool(pool);
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
            let (_, season, episode) = parse_file_stem(&file_stem);

            let entry = if ext == "mp4" {
                let collection_idx = mc.get_collection_index(&row.path)?.unwrap_or(-1);
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
                    let entries: Vec<_> = row.path.split('/').collect();
                    let len_entries = entries.len();
                    let directory = entries[len_entries-2];
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
