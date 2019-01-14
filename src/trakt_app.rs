extern crate failure;
extern crate movie_collection_rust;
extern crate rayon;

use clap::{App, Arg};
use failure::Error;
use rayon::prelude::*;

use movie_collection_rust::imdb_episodes::ImdbEpisodes;
use movie_collection_rust::imdb_ratings::ImdbRatings;
use movie_collection_rust::movie_collection::MovieCollectionDB;
use movie_collection_rust::trakt_utils::{
    get_calendar, get_watched_shows, get_watched_shows_db, get_watchlist_shows,
    get_watchlist_shows_db,
};
use movie_collection_rust::utils::{get_version_number, map_result_vec};

fn trakt_app() -> Result<(), Error> {
    let matches = App::new("Trakt Query/Parser")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Query and Parse Video Collectioin")
        .arg(
            Arg::with_name("parse")
                .short("p")
                .long("parse")
                .value_name("PARSE")
                .takes_value(false)
                .help("Parse collection for new videos"),
        )
        .arg(
            Arg::with_name("shows")
                .value_name("SHOWS")
                .help("Shows")
                .multiple(true),
        )
        .get_matches();

    let do_parse = matches.is_present("parse");

    let mq = MovieCollectionDB::new();
    if do_parse {
        let watchlist_shows_db = get_watchlist_shows_db(&mq.pool)?;
        let watchlist_shows = get_watchlist_shows()?;
        let results: Vec<Result<_, Error>> = watchlist_shows
            .par_iter()
            .map(|(link, show)| {
                if !watchlist_shows_db.contains_key(link) {
                    show.insert_show(&mq.pool)?;
                    println!("insert watchlist {}", show);
                }
                Ok(())
            })
            .collect();
        map_result_vec(results)?;

        let results: Vec<Result<_, Error>> = watchlist_shows_db
            .par_iter()
            .map(|(link, show)| {
                if !watchlist_shows.contains_key(link) {
                    show.delete_show(&mq.pool)?;
                    println!("delete watchlist {}", show);
                }
                Ok(())
            })
            .collect();
        map_result_vec(results)?;

        let watched_shows_db = get_watched_shows_db(&mq.pool)?;
        let watched_shows = get_watched_shows()?;
        let results: Vec<Result<_, Error>> = watched_shows
            .par_iter()
            .map(|(key, episode)| {
                if !watched_shows_db.contains_key(&key) {
                    episode.insert_episode(&mq.pool)?;
                    println!("insert watched {}", episode);
                }
                Ok(())
            })
            .collect();
        map_result_vec(results)?;

        let results: Vec<Result<_, Error>> = watched_shows_db
            .par_iter()
            .map(|(key, episode)| {
                if !watched_shows.contains_key(&key) {
                    episode.delete_episode(&mq.pool)?;
                    println!("delete watched {}", episode);
                }
                Ok(())
            })
            .collect();
        map_result_vec(results)?;
    } else {
        for cal in get_calendar()? {
            let show = match ImdbRatings::get_show_by_link(cal.link.clone(), &mq.pool)? {
                Some(s) => s.show,
                None => "".to_string(),
            };
            let exists = if !show.is_empty() {
                ImdbEpisodes {
                    show: show.clone(),
                    season: cal.season,
                    episode: cal.episode,
                    ..Default::default()
                }
                .get_index(&mq.pool)?
                .is_some()
            } else {
                false
            };
            if !exists {
                println!("{} {}", show, cal);
            }
        }
    }

    println!("hello world");
    Ok(())
}

fn main() {
    trakt_app().unwrap();
}
