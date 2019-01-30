extern crate actix;
extern crate actix_web;
extern crate chrono;
extern crate failure;
extern crate movie_collection_rust;
extern crate serde_json;

use chrono::{Duration, Local};
use clap::{App, Arg};
use failure::Error;
use std::io;
use std::io::Write;

use movie_collection_rust::common::movie_collection::{MovieCollection, MovieCollectionDB};
use movie_collection_rust::common::movie_queue::MovieQueueDB;
use movie_collection_rust::common::utils::get_version_number;

fn find_new_episodes() -> Result<(), Error> {
    let matches = App::new("Find new episodes")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Query and Parse Video Collectioin")
        .arg(
            Arg::with_name("source")
                .short("s")
                .long("source")
                .value_name("SOURCE")
                .takes_value(true)
                .help("Restrict source"),
        )
        .arg(
            Arg::with_name("shows")
                .value_name("SHOWS")
                .help("Shows")
                .multiple(true),
        )
        .get_matches();

    let source = matches.value_of("source").map(|s| s.to_string());
    let shows = matches
        .values_of("shows")
        .map(|v| v.map(|s| s.to_string()).collect())
        .unwrap_or_else(Vec::new);

    let source = if shows.is_empty() {
        source
    } else {
        Some("all".to_string())
    };

    let mindate = Local::today() + Duration::days(-14);
    let maxdate = Local::today() + Duration::days(7);

    let mc = MovieCollectionDB::new();
    let mq = MovieQueueDB::with_pool(&mc.pool);

    let stdout = io::stdout();

    'outer: for epi in mc.get_new_episodes(mindate.naive_local(), maxdate.naive_local(), source)? {
        for s in mq.print_movie_queue(&[&epi.show])? {
            if let Some(show) = &s.show {
                if let Some(season) = &s.season {
                    if let Some(episode) = &s.episode {
                        if (show == &epi.show)
                            && (season == &epi.season)
                            && (episode == &epi.episode)
                        {
                            continue 'outer;
                        }
                    }
                }
            }
        }
        if !shows.is_empty() && shows.iter().any(|s| &epi.show != s) {
            continue;
        }

        {
            writeln!(stdout.lock(), "{}", epi)?;
        }
    }

    Ok(())
}

fn main() {
    match find_new_episodes() {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}
