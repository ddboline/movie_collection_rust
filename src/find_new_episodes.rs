extern crate actix;
extern crate actix_web;
extern crate chrono;
extern crate failure;
extern crate movie_collection_rust;
extern crate serde_json;

use chrono::{Duration, Local};
use clap::{App, Arg};
use failure::Error;

use movie_collection_rust::common::movie_collection::MovieCollectionDB;
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

    let mindate = Local::today() + Duration::days(-14);
    let maxdate = Local::today() + Duration::days(7);

    let mc = MovieCollectionDB::new();
    let mq = MovieQueueDB::with_pool(mc.pool.clone());

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
        println!("{}", epi);
    }

    Ok(())
}

fn main() {
    find_new_episodes().unwrap()
}
