extern crate chrono;
extern crate clap;
extern crate failure;
extern crate movie_collection_rust;

use chrono::NaiveDate;
use clap::{App, Arg};
use failure::Error;
use std::collections::HashMap;

use movie_collection_rust::movie_collection::{
    parse_imdb, parse_imdb_episode_list, ImdbEpisodes, ImdbRatings, MovieCollection,
};
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

    let mq = MovieCollection::new();

    let shows: HashMap<String, _> = if let Some(ilink) = &imdb_link {
        mq.print_imdb_shows(show, tv)?
            .into_iter()
            .filter_map(|s| match &s.link {
                Some(l) if l == ilink => Some((l.clone(), s.clone())),
                _ => None,
            })
            .collect()
    } else {
        mq.print_imdb_shows(show, tv)?
            .into_iter()
            .filter_map(|s| match &s.link {
                Some(l) => Some((l.clone(), s.clone())),
                None => None,
            })
            .collect()
    };

    let episodes: Option<HashMap<(i32, i32), _>> = if tv {
        if all_seasons {
            mq.print_imdb_all_seasons(show)?;
            None
        } else {
            let r = mq
                .print_imdb_episodes(show, season)?
                .into_iter()
                .map(|e| ((e.season, e.episode), e))
                .collect();
            Some(r)
        }
    } else {
        None
    };

    if do_update {
        let results = parse_imdb(&show.replace("_", " "))?;
        let results = if let Some(ilink) = &imdb_link {
            results.into_iter().filter(|r| &r.link == ilink).collect()
        } else {
            results
        };

        let link = if let Some(link) = imdb_link {
            Some(link)
        } else if let Some(result) = results.get(0) {
            Some(result.link.clone())
        } else {
            None
        };

        if !tv {
            if update_database {
                if let Some(result) = results.get(0) {
                    match shows.get(&result.link) {
                        Some(s) => {
                            if (result.rating - s.rating.unwrap_or(-1.0)).abs() > 0.1 {
                                let mut new = s.clone();
                                new.title = Some(result.title.clone());
                                new.rating = Some(result.rating);
                                mq.update_imdb_show(&new)?;
                                println!("exists {} {} {}", show, s, result.rating);
                            }
                        }
                        None => {
                            println!("not exists {} {}", show, result);
                            let istv = result.title.contains("TV Series")
                                || result.title.contains("TV Mini-Series");
                            mq.insert_imdb_show(&ImdbRatings {
                                show: show.to_string(),
                                title: Some(result.title.clone()),
                                link: Some(result.link.clone()),
                                rating: Some(result.rating),
                                istv: Some(istv),
                                ..Default::default()
                            })?;
                        }
                    }
                }
            }

            for result in &results {
                println!("{}", result);
            }
        } else if let Some(link) = link {
            if let Some(result) = shows.get(&link) {
                for episode in parse_imdb_episode_list(&link, season)? {
                    println!("{} {}", result, episode);
                    if update_database {
                        let key = (episode.season, episode.episode);
                        if let Some(episodes) = &episodes {
                            match episodes.get(&key) {
                                Some(e) => {
                                    if (e.rating - episode.rating.unwrap_or(-1.0)).abs() > 0.1 {
                                        println!("exists {} {} {}", result, episode, e.rating);
                                        let mut new = e.clone();
                                        new.eptitle =
                                            episode.eptitle.unwrap_or_else(|| "".to_string());
                                        new.rating = episode.rating.unwrap_or(-1.0);
                                        mq.update_imdb_episodes(&new)?;
                                    }
                                }
                                None => {
                                    println!("not exists {} {}", result, episode);
                                    mq.insert_imdb_episode(&ImdbEpisodes {
                                        show: show.to_string(),
                                        title: result
                                            .title
                                            .clone()
                                            .unwrap_or_else(|| "".to_string()),
                                        season: episode.season,
                                        episode: episode.episode,
                                        airdate: episode
                                            .airdate
                                            .unwrap_or_else(|| NaiveDate::from_ymd(1970, 1, 1)),
                                        rating: episode.rating.unwrap_or(-1.0),
                                        eptitle: episode.eptitle.unwrap_or_else(|| "".to_string()),
                                        epurl: episode.epurl.unwrap_or_else(|| "".to_string()),
                                    })?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn main() {
    parse_imdb_parser().unwrap();
}
