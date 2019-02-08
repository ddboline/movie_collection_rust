extern crate chrono;
extern crate clap;
extern crate failure;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::Error;
use std::io::{stdout, Write};

use movie_collection_rust::common::movie_collection::MovieCollectionDB;
use movie_collection_rust::common::parse_imdb::ParseImdb;
use movie_collection_rust::common::utils::get_version_number;

fn parse_imdb_parser() -> Result<(), Error> {
    let matches = App::new("Parse IMDB")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Parse IMDB.com")
        .arg(
            Arg::with_name("tv")
                .short("t")
                .long("tv")
                .value_name("TELEVISION")
                .takes_value(false)
                .help("Is Television"),
        )
        .arg(
            Arg::with_name("season")
                .short("s")
                .long("season")
                .value_name("SEASON")
                .takes_value(true)
                .help("Season number"),
        )
        .arg(
            Arg::with_name("all_seasons")
                .short("a")
                .long("all_seasons")
                .takes_value(false)
                .help("List Seasons"),
        )
        .arg(
            Arg::with_name("update")
                .short("u")
                .long("update")
                .value_name("UPDATE")
                .takes_value(false)
                .help("Do update"),
        )
        .arg(
            Arg::with_name("imdblink")
                .short("i")
                .long("imdblink")
                .takes_value(true)
                .help("Manually override imdb link"),
        )
        .arg(
            Arg::with_name("database")
                .short("d")
                .long("database")
                .value_name("DATABASE")
                .takes_value(false)
                .help("Update Database"),
        )
        .arg(Arg::with_name("show").value_name("SHOW").help("Show"))
        .get_matches();

    let show = matches.value_of("show").unwrap_or("");
    let tv = matches.is_present("tv");
    let imdb_link = matches.value_of("imdblink").map(|s| s.to_string());

    let all_seasons = matches.is_present("all_seasons");

    let season: Option<i32> = if let Some(s) = matches.value_of("season") {
        Some(s.parse()?)
    } else {
        None
    };

    let do_update = matches.is_present("update");
    let update_database = matches.is_present("database");

    let mc = MovieCollectionDB::new();
    let pi = ParseImdb::with_pool(&mc.pool);

    let output = pi.parse_imdb_worker(
        show,
        tv,
        imdb_link,
        all_seasons,
        season,
        do_update,
        update_database,
    )?;

    let stdout = stdout();

    for line in output {
        writeln!(stdout.lock(), "{}", line.join(" "))?;
    }

    Ok(())
}

fn main() {
    match parse_imdb_parser() {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}
