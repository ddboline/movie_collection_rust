use clap::{App, Arg};
use failure::Error;
use std::io;
use std::io::Write;

use movie_collection_rust::common::movie_collection::{MovieCollection, MovieCollectionDB};
use movie_collection_rust::common::tv_show_source::TvShowSource;
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

    let source: Option<TvShowSource> = match source {
        Some(s) => s.parse().ok(),
        None => None,
    };

    let source = if shows.is_empty() {
        source
    } else {
        Some(TvShowSource::All)
    };

    let mc = MovieCollectionDB::new();

    let output = mc.find_new_episodes(&source, &shows)?;

    let stdout = io::stdout();

    for epi in output {
        writeln!(stdout.lock(), "{}", epi)?;
    }

    Ok(())
}

fn main() {
    env_logger::init();

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
