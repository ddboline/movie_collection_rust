extern crate chrono;
extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate rayon;
extern crate reqwest;
extern crate select;

use chrono::NaiveDate;
use failure::Error;
use std::collections::HashMap;

use crate::common::imdb_episodes::ImdbEpisodes;
use crate::common::imdb_ratings::ImdbRatings;
use crate::common::imdb_utils::ImdbConnection;
use crate::common::movie_collection::MovieCollection;

pub fn parse_imdb_worker(
    mc: &MovieCollection,
    show: &str,
    tv: bool,
    imdb_link: Option<String>,
    all_seasons: bool,
    season: Option<i32>,
    do_update: bool,
    update_database: bool,
) -> Result<Vec<Vec<String>>, Error> {
    let shows: Vec<_> = if let Some(ilink) = &imdb_link {
        mc.print_imdb_shows(show, tv)?
            .into_iter()
            .filter_map(|s| match &s.link {
                Some(l) if l == ilink => Some((l.clone(), s)),
                _ => None,
            })
            .collect()
    } else {
        mc.print_imdb_shows(show, tv)?
            .into_iter()
            .filter_map(|s| match &s.link {
                Some(l) => Some((l.clone(), s)),
                None => None,
            })
            .collect()
    };

    let mut output = Vec::new();

    if !do_update {
        for (_, s) in &shows {
            output.push(s.get_string_vec());
        }
    }

    let shows: HashMap<String, _> = shows.into_iter().collect();

    let episodes: Option<Vec<_>> = if tv {
        if all_seasons {
            if !do_update {
                for s in mc.print_imdb_all_seasons(show)? {
                    output.push(s.get_string_vec());
                }
            }
            None
        } else {
            let r = mc
                .print_imdb_episodes(show, season)?
                .into_iter()
                .map(|e| ((e.season, e.episode), e))
                .collect();
            Some(r)
        }
    } else {
        None
    };

    if !do_update {
        if let Some(v) = episodes.as_ref() {
            for ((_, _), e) in v {
                output.push(e.get_string_vec());
            }
        }
    }

    let episodes: Option<HashMap<(i32, i32), _>> = episodes.map(|v| v.into_iter().collect());

    if do_update {
        let imdb_conn = ImdbConnection::new();
        let results = imdb_conn.parse_imdb(&show.replace("_", " "))?;
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
                                new.update_show(&mc.get_pool())?;
                                output
                                    .push(vec![format!("exists {} {} {}", show, s, result.rating)]);
                            }
                        }
                        None => {
                            output.push(vec![format!("not exists {} {}", show, result)]);
                            let istv = result.title.contains("TV Series")
                                || result.title.contains("TV Mini-Series");

                            ImdbRatings {
                                show: show.to_string(),
                                title: Some(result.title.clone()),
                                link: Some(result.link.clone()),
                                rating: Some(result.rating),
                                istv: Some(istv),
                                ..Default::default()
                            }
                            .insert_show(&mc.get_pool())?;
                        }
                    }
                }
            }

            for result in &results {
                output.push(vec![format!("{}", result)]);
            }
        } else if let Some(link) = link {
            output.push(vec![format!("Using {}", link)]);
            if let Some(result) = shows.get(&link) {
                for episode in imdb_conn.parse_imdb_episode_list(&link, season)? {
                    output.push(vec![format!("{} {}", result, episode)]);
                    if update_database {
                        let key = (episode.season, episode.episode);
                        if let Some(episodes) = &episodes {
                            match episodes.get(&key) {
                                Some(e) => {
                                    if (e.rating - episode.rating.unwrap_or(-1.0)).abs() > 0.1 {
                                        output.push(vec![format!(
                                            "exists {} {} {}",
                                            result, episode, e.rating
                                        )]);
                                        let mut new = e.clone();
                                        new.eptitle =
                                            episode.eptitle.unwrap_or_else(|| "".to_string());
                                        new.rating = episode.rating.unwrap_or(-1.0);
                                        new.update_episode(&mc.get_pool())?;
                                    }
                                }
                                None => {
                                    output.push(vec![format!("not exists {} {}", result, episode)]);
                                    ImdbEpisodes {
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
                                    }
                                    .insert_episode(&mc.get_pool())?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(output)
}
