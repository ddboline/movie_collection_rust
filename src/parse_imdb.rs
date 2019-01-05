extern crate clap;
extern crate failure;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::Error;

use movie_collection_rust::movie_collection::MovieCollection;
use movie_collection_rust::utils::get_version_number;

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
        .arg(Arg::with_name("show").value_name("SHOW").help("Show"))
        .get_matches();

    let show = matches.value_of("show").unwrap_or("");
    let tv = matches.is_present("tv");

    let all_seasons = matches.is_present("all_seasons");

    let season: Option<i64> = if let Some(s) = matches.value_of("season") {
        Some(s.parse()?)
    } else {
        None
    };

    let do_update = matches.is_present("update");

    let mq = MovieCollection::new();

    mq.print_imdb_shows(show, tv)?;

    if tv {
        if all_seasons {
            mq.print_imdb_all_seasons(show)?;
        } else {
            mq.print_imdb_episodes(show, season)?;
        }
    }

    Ok(())
}

fn main() {
    parse_imdb_parser().unwrap();
}
