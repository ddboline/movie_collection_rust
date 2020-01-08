use anyhow::Error;
use chrono::NaiveDate;
use std::collections::HashMap;

use crate::imdb_episodes::ImdbEpisodes;
use crate::imdb_ratings::ImdbRatings;
use crate::imdb_utils::ImdbConnection;
use crate::movie_collection::MovieCollection;
use crate::pgpool::PgPool;
use crate::trakt_utils::WatchListMap;

#[derive(Default, Debug)]
pub struct ParseImdbOptions {
    pub show: String,
    pub tv: bool,
    pub imdb_link: Option<String>,
    pub all_seasons: bool,
    pub season: Option<i32>,
    pub do_update: bool,
    pub update_database: bool,
}

#[derive(Default)]
pub struct ParseImdb {
    pub mc: MovieCollection,
}

impl ParseImdb {
    pub fn with_pool(pool: &PgPool) -> Result<Self, Error> {
        let p = Self {
            mc: MovieCollection::with_pool(&pool)?,
        };
        Ok(p)
    }

    pub fn parse_imdb_worker(&self, opts: &ParseImdbOptions) -> Result<Vec<Vec<String>>, Error> {
        let shows: Vec<_> = if let Some(ilink) = &opts.imdb_link {
            self.mc
                .print_imdb_shows(&opts.show, opts.tv)?
                .into_iter()
                .filter_map(|s| {
                    if &s.link == ilink {
                        Some((s.link.to_string(), s))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            self.mc
                .print_imdb_shows(&opts.show, opts.tv)?
                .into_iter()
                .map(|s| (s.link.to_string(), s))
                .collect()
        };

        let mut output = Vec::new();

        if !opts.do_update {
            for (_, s) in &shows {
                output.push(s.get_string_vec());
            }
        }

        let shows: HashMap<String, _> = shows.into_iter().collect();

        let episodes: Option<Vec<_>> = if opts.tv {
            if opts.all_seasons {
                if !opts.do_update {
                    let seasons = self.mc.print_imdb_all_seasons(&opts.show)?;
                    for s in seasons {
                        output.push(s.get_string_vec());
                    }
                }
                None
            } else {
                let r = self
                    .mc
                    .print_imdb_episodes(&opts.show, opts.season)?
                    .into_iter()
                    .map(|e| ((e.season, e.episode), e))
                    .collect();
                Some(r)
            }
        } else {
            None
        };

        if !opts.do_update {
            if let Some(v) = episodes.as_ref() {
                for ((_, _), e) in v {
                    output.push(e.get_string_vec());
                }
            }
        }

        let episodes: Option<HashMap<(i32, i32), _>> = episodes.map(|v| v.into_iter().collect());

        if opts.do_update {
            self.parse_imdb_update_worker(&opts, &shows, &episodes, &mut output)?;
        }
        Ok(output)
    }

    fn parse_imdb_update_worker(
        &self,
        opts: &ParseImdbOptions,
        shows: &HashMap<String, ImdbRatings>,
        episodes: &Option<HashMap<(i32, i32), ImdbEpisodes>>,
        output: &mut Vec<Vec<String>>,
    ) -> Result<(), Error> {
        let imdb_conn = ImdbConnection::new();
        let results = imdb_conn.parse_imdb(&opts.show.replace("_", " "))?;
        let results = if let Some(ilink) = &opts.imdb_link {
            results.into_iter().filter(|r| &r.link == ilink).collect()
        } else {
            results
        };

        let link = if let Some(link) = &opts.imdb_link {
            Some(link.to_string())
        } else if let Some(result) = results.get(0) {
            Some(result.link.to_string())
        } else {
            None
        };

        if !opts.tv {
            if opts.update_database {
                if let Some(result) = results.get(0) {
                    match shows.get(&result.link) {
                        Some(s) => {
                            if (result.rating - s.rating.unwrap_or(-1.0)).abs() > 0.1 {
                                let mut new = s.clone();
                                new.title = Some(result.title.to_string());
                                new.rating = Some(result.rating);
                                new.update_show(&self.mc.get_pool())?;
                                output.push(vec![format!(
                                    "exists {} {} {}",
                                    opts.show, s, result.rating
                                )]);
                            }
                        }
                        None => {
                            output.push(vec![format!("not exists {} {}", opts.show, result)]);
                            let istv = result.title.contains("TV Series")
                                || result.title.contains("TV Mini-Series");

                            ImdbRatings {
                                show: opts.show.to_string(),
                                title: Some(result.title.to_string()),
                                link: result.link.to_string(),
                                rating: Some(result.rating),
                                istv: Some(istv),
                                ..ImdbRatings::default()
                            }
                            .insert_show(&self.mc.get_pool())?;
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
                let episode_list = imdb_conn.parse_imdb_episode_list(&link, opts.season)?;
                for episode in episode_list {
                    output.push(vec![format!("{} {}", result, episode)]);
                    if opts.update_database {
                        let key = (episode.season, episode.episode);
                        if let Some(episodes) = &episodes {
                            let airdate = episode
                                .airdate
                                .unwrap_or_else(|| NaiveDate::from_ymd(1970, 1, 1));
                            match episodes.get(&key) {
                                Some(e) => {
                                    if (e.rating - episode.rating.unwrap_or(-1.0)).abs() > 0.1
                                        || e.airdate != airdate
                                    {
                                        output.push(vec![format!(
                                            "exists {} {} {}",
                                            result, episode, e.rating
                                        )]);
                                        let mut new = e.clone();
                                        new.eptitle =
                                            episode.eptitle.unwrap_or_else(|| "".to_string());
                                        new.rating = episode.rating.unwrap_or(-1.0);
                                        new.airdate = airdate;
                                        new.update_episode(&self.mc.get_pool())?;
                                    }
                                }
                                None => {
                                    output.push(vec![format!("not exists {} {}", result, episode)]);
                                    ImdbEpisodes {
                                        show: opts.show.to_string(),
                                        title: result
                                            .title
                                            .clone()
                                            .unwrap_or_else(|| "".to_string()),
                                        season: episode.season,
                                        episode: episode.episode,
                                        airdate,
                                        rating: episode.rating.unwrap_or(-1.0),
                                        eptitle: episode.eptitle.unwrap_or_else(|| "".to_string()),
                                        epurl: episode.epurl.unwrap_or_else(|| "".to_string()),
                                    }
                                    .insert_episode(&self.mc.get_pool())?;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn parse_imdb_http_worker(
        &self,
        opts: &ParseImdbOptions,
        watchlist: &WatchListMap,
    ) -> Result<String, Error> {
        let button_add = format!(
            "{}{}",
            r#"<td><button type="submit" id="ID" "#,
            r#"onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#
        );
        let button_rm = format!(
            "{}{}",
            r#"<td><button type="submit" id="ID" "#,
            r#"onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#
        );

        let output: Vec<_> = self
            .parse_imdb_worker(&opts)?
            .into_iter()
            .map(|line| {
                let mut imdb_url = "".to_string();
                let tmp: Vec<_> = line
                    .into_iter()
                    .map(|i| {
                        if i.starts_with("tt") {
                            imdb_url = i.to_string();
                            format!(r#"<a href="https://www.imdb.com/title/{}">{}</a>"#, i, i)
                        } else {
                            i
                        }
                    })
                    .collect();
                format!(
                    "<tr><td>{}</td><td>{}</td></tr>",
                    tmp.join("</td><td>"),
                    if watchlist.contains_key(&imdb_url) {
                        button_rm.replace("SHOW", &imdb_url)
                    } else {
                        button_add.replace("SHOW", &imdb_url)
                    }
                )
            })
            .collect();

        Ok(output.join("\n"))
    }
}
