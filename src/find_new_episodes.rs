#[macro_use]
extern crate serde_derive;

extern crate actix;
extern crate actix_web;
extern crate chrono;
extern crate failure;
extern crate movie_collection_rust;
extern crate rayon;
extern crate serde_json;

use chrono::Local;
use failure::Error;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

use movie_collection_rust::config::Config;
use movie_collection_rust::movie_collection::MovieCollection;
use movie_collection_rust::utils::{
    get_calendar, get_watched_shows, get_watchlist_shows, map_result_vec, option_string_wrapper,
};

fn find_new_episodes() -> Result<(), Error> {
    let trakt_watchlist_shows = get_watchlist_shows()?;
    println!("watchlist {}", trakt_watchlist_shows.len());
    let trakt_watched_shows = get_watched_shows()?;
    println!("watched {}", trakt_watched_shows.len());
    let trakt_cal_shows: HashMap<String, String> = get_calendar()?
        .into_iter()
        .map(|row| (row.link, row.show))
        .collect();

    println!("cal {}", trakt_cal_shows.len());

    let mq = MovieCollection::new();

    let mut max_season_map = HashMap::new();
    let mut max_episode_map = HashMap::new();
    let imdb_show_map = mq.get_imdb_show_map()?;
    let mut current_shows: HashSet<String> = mq
        .print_movie_queue(&[])?
        .iter()
        .filter_map(|row| {
            if let Some(show) = &row.show {
                let max_season = max_season_map.get(show).unwrap_or(&-1);
                if let Some(season) = row.season {
                    if season > *max_season {
                        max_season_map.insert(show.clone(), season);
                    }
                    let key = (show.clone(), season);
                    let max_episode = max_episode_map.get(&key).unwrap_or(&-1);
                    if let Some(episode) = row.episode {
                        if episode > *max_episode {
                            max_episode_map.insert(key, episode);
                        }
                    }
                }
                if let Some(link) = &row.link {
                    Some(link.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    let maxdate = Local::today();

    for (k, v) in trakt_watchlist_shows.iter() {
        if !current_shows.contains(k) {
            if !imdb_show_map.contains_key(k) {
                panic!("not in current_shows {} {}", k, v.title);
            } else {
                current_shows.insert(k.clone());
            }
        }
    }
/*
    println!("current_shows {}", current_shows.len());
    println!("max_season_map {}", max_season_map.len());
    println!("max_episode_map {}", max_episode_map.len());

    let mut current_seasons: HashMap<String, Vec<i32>> = HashMap::new();
    let mut current_episodes: HashMap<String, Vec<(i32, i32)>> = HashMap::new();

    for imdb_url in &current_shows {
        if let Some(watch_map) = trakt_watched_shows.get(imdb_url) {
            let show = &imdb_show_map.get(imdb_url).as_ref().unwrap().show;
            println!("{} {} {}", show, imdb_url, watch_map.len());

            for (key, episode_info) in watch_map {
                println!("episode_info {:?}", episode_info);
                let key: Vec<i32> = key
                    .replace("(", "")
                    .replace(")", "")
                    .split(", ")
                    .map(|s| s.parse().unwrap())
                    .collect();
                let s = key[0];
                let e = key[1];
                let max_s = max_season_map.get(imdb_url).unwrap_or(&-1);
                let max_e = max_episode_map.get(&(show.clone(), s)).unwrap_or(&-1);
                if s > *max_s {
                    max_season_map.insert(imdb_url.clone(), s);
                }
                if e > *max_e {
                    max_episode_map.insert((show.clone(), s), e);
                }
                if let Some(m) = current_seasons.get_mut(imdb_url) {
                    m.push(s);
                } else {
                    current_seasons.insert(imdb_url.clone(), vec![s]);
                }
                if let Some(m) = current_episodes.get_mut(imdb_url) {
                    m.push((s, e));
                } else {
                    current_episodes.insert(imdb_url.clone(), vec![(s, e)]);
                }
            }
        }
    }

    println!("current_seasons {}", current_seasons.len());
    println!("current_episodes {}", current_episodes.len());

    for imdb_url in &current_shows {
        let show_info = imdb_show_map.get(imdb_url).unwrap();
        let show = &show_info.show;
        println!("show {} imdb_url {}", show, imdb_url);
        let max_s = max_season_map.get(imdb_url).unwrap_or(&-1);
        let max_e = max_episode_map
            .get(&(imdb_url.to_string(), *max_s))
            .unwrap_or(&-1);
        let title = option_string_wrapper(&show_info.title);
        let rating = show_info.rating.unwrap_or(-1.0);
        let source = option_string_wrapper(&show_info.source);
        println!("{} {:?} {:?} {:?}", show, title, rating, source);
    }
*/
    Ok(())
}

/*
        if (source != 'all' and source in ('hulu', 'netflix', 'amazon') and
                mq_.imdb_ratings[show]['source'] != source):
            continue
        if not source and mq_.imdb_ratings[show]['source'] in ('hulu', 'netflix', 'amazon'):
            continue

        max_airdate = datetime.date(1950, 1, 1)

        if mq_.imdb_episode_ratings[show]:
            max_s, max_e = max(mq_.imdb_episode_ratings[show])
            max_airdate = mq_.imdb_episode_ratings[show][(max_s, max_e)]['airdate']

        if shows:
            output[show] = '%s %s %s %s %s %s' % (show, title, max_s, max_e, str(max_airdate),
                                                  rating)
            continue
        if do_update:
            if max_airdate > datetime.date.today() - datetime.timedelta(days=30):
                print(show, max_s, max_e)
                for item in parse_imdb_episode_list(imdb_url, season=-1):
                    season = item[0]
                    if season < max_s:
                        continue
                    mq_.get_imdb_episode_ratings(show, season)
        for season, episode in sorted(mq_.imdb_episode_ratings[show]):
            row = mq_.imdb_episode_ratings[show][(season, episode)]
            if season < max_s:
                continue
            if episode <= max_episode[imdb_url].get(season, -1):
                continue
            if not search and row['airdate'] < (maxdate - datetime.timedelta(days=10)):
                continue
            if row['airdate'] > maxdate:
                continue
            if (season, episode) in current_episodes[imdb_url]:
                continue
            eptitle = row['eptitle']
            eprating = row['rating']
            airdate = row['airdate']
            output[(airdate,
                    show)] = '%s %s %s %d %d %0.2f/%0.2f %s' % (show, title, eptitle, season,
                                                                episode, eprating, rating, airdate)
    for key in sorted(output):
        val = output[key]
        print(val)

*/

fn main() {
    find_new_episodes().unwrap()
}
