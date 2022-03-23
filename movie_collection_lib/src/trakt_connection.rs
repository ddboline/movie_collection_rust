use anyhow::{format_err, Error};
use base64::{encode_config, URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use log::debug;
use maplit::hashmap;
use rand::{thread_rng, Rng};
use reqwest::{header::HeaderMap, Client, Url};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    fs::{read, write},
    sync::{Mutex, RwLock},
};

use crate::{
    config::Config,
    iso_8601_datetime,
    trakt_utils::{
        TraktCalEntry, TraktCalEntryList, TraktResult, WatchListShow, WatchedEpisode, WatchedMovie,
    },
};

lazy_static! {
    static ref CSRF_TOKEN: Mutex<Option<StackString>> = Mutex::new(None);
    static ref AUTH_TOKEN: RwLock<Option<Arc<AccessTokenResponse>>> = RwLock::new(None);
}

#[derive(Clone)]
pub struct TraktConnection {
    config: Config,
    client: Client,
}

impl Default for TraktConnection {
    fn default() -> Self {
        let config = Config::with_config().expect("Failed to create");
        Self::new(config)
    }
}

impl TraktConnection {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    #[must_use]
    pub fn get_client(&self) -> &Client {
        &self.client
    }

    /// # Errors
    /// Return error if `read_auth_token` fails
    pub async fn init(&self) {
        if let Ok(auth_token) = self.read_auth_token().await {
            AUTH_TOKEN.write().await.replace(Arc::new(auth_token));
        } else {
            println!("read_auth_token failed...");
        }
    }

    fn token_path() -> Result<PathBuf, Error> {
        let home_dir = dirs::home_dir().ok_or_else(|| format_err!("No home dir"))?;
        Ok(home_dir.join(".trakt").join("auth_token_web.json"))
    }

    async fn read_auth_token(&self) -> Result<AccessTokenResponse, Error> {
        serde_json::from_slice(&read(Self::token_path()?).await?).map_err(Into::into)
    }

    async fn write_auth_token(&self, token: &AccessTokenResponse) -> Result<(), Error> {
        write(&Self::token_path()?, &serde_json::to_vec(token)?)
            .await
            .map_err(Into::into)
    }

    fn get_random_string() -> String {
        let random_bytes: SmallVec<[u8; 16]> = (0..16).map(|_| thread_rng().gen::<u8>()).collect();
        encode_config(&random_bytes, URL_SAFE_NO_PAD)
    }

    fn _get_auth_url(&self, state: &str) -> Result<Url, Error> {
        let domain = &self.config.domain;
        let redirect_uri = format_sstr!("https://{domain}/trakt/callback");
        let parameters = &[
            ("response_type", "code"),
            ("client_id", self.config.trakt_client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("state", state),
        ];
        Url::parse_with_params("https://trakt.tv/oauth/authorize", parameters).map_err(Into::into)
    }

    /// # Errors
    /// Return error if parsing url fails
    pub async fn get_auth_url(&self) -> Result<Url, Error> {
        let state = Self::get_random_string();
        let url = self._get_auth_url(&state)?;
        CSRF_TOKEN.lock().await.replace(state.into());
        Ok(url)
    }

    async fn get_auth_token(&self, code: &str, state: &str) -> Result<AccessTokenResponse, Error> {
        let current_state = CSRF_TOKEN.lock().await.take();
        if let Some(current_state) = current_state {
            if state != current_state.as_str() {
                return Err(format_err!("Incorrect state"));
            }
            let domain = &self.config.domain;
            let redirect_uri = format_sstr!("https://{domain}/trakt/callback");
            let trakt_endpoint = &self.config.trakt_endpoint;
            let url = format_sstr!("{trakt_endpoint}/oauth/token");
            let body = hashmap! {
                "code" => code,
                "client_id" => self.config.trakt_client_id.as_str(),
                "client_secret" => self.config.trakt_client_secret.as_str(),
                "redirect_uri" => redirect_uri.as_str(),
                "grant_type" => "authorization_code",
            };
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", "application/json".parse()?);
            self.client
                .post(url.as_str())
                .headers(headers)
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
                .map_err(Into::into)
        } else {
            Err(format_err!("No state"))
        }
    }

    async fn get_refresh_token(&self) -> Result<AccessTokenResponse, Error> {
        let current_auth_token = AUTH_TOKEN.read().await.clone();
        if let Some(current_auth_token) = current_auth_token {
            let domain = &self.config.domain;
            let redirect_uri = format_sstr!("https://{domain}/trakt/callback");
            let trakt_endpoint = &self.config.trakt_endpoint;
            let url = format_sstr!("{trakt_endpoint}/oauth/token");
            let body = hashmap! {
                "refresh_token" => current_auth_token.refresh_token.as_str(),
                "client_id" => self.config.trakt_client_id.as_str(),
                "client_secret" => self.config.trakt_client_secret.as_str(),
                "redirect_uri" => redirect_uri.as_str(),
                "grant_type" => "refresh_token",
            };
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", "application/json".parse()?);
            self.client
                .post(url.as_str())
                .headers(headers)
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
                .map_err(Into::into)
        } else {
            Err(format_err!("No refresh_token"))
        }
    }

    /// # Errors
    /// Return error if `get_auth_token` or `write_auth_token` fail
    pub async fn exchange_code_for_auth_token(&self, code: &str, state: &str) -> Result<(), Error> {
        let auth_token = self.get_auth_token(code, state).await?;
        self.write_auth_token(&auth_token).await?;
        AUTH_TOKEN.write().await.replace(Arc::new(auth_token));
        Ok(())
    }

    /// # Errors
    /// Return error if `get_refresh_token` or `write_auth_token` fail
    pub async fn exchange_refresh_token(&self) -> Result<(), Error> {
        let auth_token = self.get_refresh_token().await?;
        self.write_auth_token(&auth_token).await?;
        AUTH_TOKEN.write().await.replace(Arc::new(auth_token));
        Ok(())
    }

    fn get_ro_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        headers.insert("Content-type", "application/json".parse()?);
        headers.insert("trakt-api-key", self.config.trakt_client_id.parse()?);
        headers.insert("trakt-api-version", "2".parse()?);
        Ok(headers)
    }

    async fn get_rw_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = self.get_ro_headers()?;
        let auth_token = AUTH_TOKEN
            .read()
            .await
            .clone()
            .ok_or_else(|| format_err!("No auth token"))?;
        let access_token = &auth_token.access_token;
        let bearer = format_sstr!("Bearer {access_token}");
        headers.insert("Authorization", bearer.parse()?);
        Ok(headers)
    }

    async fn get_watchlist_shows_page(
        &self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<WatchListShowsResponse>, Error> {
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watchlist/shows");
        let page_str = StackString::from_display(page);
        let limit_str = StackString::from_display(limit);
        let url = Url::parse_with_params(&url, &[("page", &page_str), ("limit", &limit_str)])?;
        let resp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        let headers = resp.headers();
        if let Some(current_page) = headers.get("X-Pagination-Page") {
            let current_page: usize = current_page.to_str()?.parse()?;
            assert_eq!(current_page, page);
        }
        resp.json().await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if `get_watchlist_shows_page` fails
    pub async fn get_watchlist_shows(&self) -> Result<HashMap<StackString, WatchListShow>, Error> {
        let mut current_page = 1;
        let mut results = Vec::new();
        loop {
            let page = self.get_watchlist_shows_page(current_page, 20).await?;
            current_page += 1;
            if page.is_empty() {
                break;
            }
            results.extend_from_slice(&page);
        }
        let watchlist = results
            .into_iter()
            .filter_map(|r| {
                let imdb: StackString = r.show.ids.imdb.unwrap_or_else(|| "".into());
                let title = r.show.title;
                r.show.year.map(|year| {
                    (
                        imdb.clone(),
                        WatchListShow {
                            link: imdb,
                            title,
                            year,
                            ..WatchListShow::default()
                        },
                    )
                })
            })
            .collect();
        Ok(watchlist)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_show_by_imdb_id(
        &self,
        imdb_id: &str,
    ) -> Result<Vec<TraktShowSearchResponse>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/search/imdb/{imdb_id}?type=show");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_movie_by_imdb_id(
        &self,
        imdb_id: &str,
    ) -> Result<Vec<TraktMovieSearchResponse>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/search/imdb/{imdb_id}?type=movie");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_episode(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktEpisodeObject, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url =
            format_sstr!("{trakt_endpoint}/shows/{imdb_id}/seasons/{season}/episodes/{episode}");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn add_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let show_obj = self
            .get_show_by_imdb_id(imdb_id)
            .await?
            .pop()
            .ok_or_else(|| format_err!("No show returned"))?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watchlist");
        let data = hashmap! {
            "shows" => vec![show_obj.show],
        };
        debug!("shows: {}", serde_json::to_string_pretty(&data)?);
        let text = self
            .client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(TraktResult {
            status: text.into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn remove_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let show_obj = self
            .get_show_by_imdb_id(imdb_id)
            .await?
            .pop()
            .ok_or_else(|| format_err!("No show returned"))?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watchlist/remove");
        let data = hashmap! {
            "shows" => vec![show_obj.show],
        };
        let text = self
            .client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(TraktResult {
            status: text.into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_watched_shows(
        &self,
    ) -> Result<HashMap<(StackString, i32, i32), WatchedEpisode>, Error> {
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watched/shows");
        let watched_episodes: Vec<TraktWatchedShowResponse> = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        #[allow(clippy::manual_filter_map)]
        let episode_map = watched_episodes
            .into_iter()
            .filter(|show_entry| show_entry.show.ids.imdb.is_some())
            .flat_map(|show_entry| {
                let title = show_entry.show.title.clone();
                let imdb_url: StackString = show_entry
                    .show
                    .ids
                    .imdb
                    .as_ref()
                    .map_or_else(|| "".into(), Clone::clone);
                show_entry
                    .seasons
                    .into_iter()
                    .flat_map(move |season_entry| {
                        let season = season_entry.number;
                        let title = title.clone();
                        let imdb_url = imdb_url.clone();
                        season_entry.episodes.into_iter().map(move |episode_entry| {
                            let episode = episode_entry.number;
                            let epi = WatchedEpisode {
                                title: title.clone(),
                                imdb_url: imdb_url.clone(),
                                episode,
                                season,
                                ..WatchedEpisode::default()
                            };
                            ((imdb_url.clone(), season, episode), epi)
                        })
                    })
            })
            .collect();
        Ok(episode_map)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_watched_movies(&self) -> Result<HashSet<WatchedMovie>, Error> {
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watched/movies");
        let watched_movies: Vec<TraktWatchedMovieResponse> = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let movie_map: HashSet<WatchedMovie> = watched_movies
            .into_iter()
            .map(|entry| {
                let imdb: StackString = entry
                    .movie
                    .ids
                    .imdb
                    .as_ref()
                    .map_or_else(|| "".into(), Clone::clone);
                WatchedMovie {
                    title: entry.movie.title,
                    imdb_url: imdb,
                }
            })
            .collect();
        Ok(movie_map)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_calendar(&self) -> Result<TraktCalEntryList, Error> {
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/calendars/my/shows");
        let new_episodes: Vec<TraktCalendarResponse> = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let cal_entries = new_episodes
            .into_iter()
            .map(|entry| {
                let imdb: StackString = entry.show.ids.imdb.unwrap_or_else(|| "".into());
                TraktCalEntry {
                    ep_link: entry.episode.ids.imdb.as_ref().map(Clone::clone),
                    episode: entry.episode.number,
                    link: imdb,
                    season: entry.episode.season,
                    show: entry.show.title,
                    airdate: entry.first_aired.naive_local().date(),
                }
            })
            .collect();
        Ok(cal_entries)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn add_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let episode_obj = self.get_episode(imdb_id, season, episode).await?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history");
        let data = hashmap! {
            "episodes" => vec![
                WatchedEpisodeRequest {
                    watched_at: Utc::now(),
                    ids: episode_obj.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success".into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn add_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let movie_obj = {
            self.get_movie_by_imdb_id(imdb_id)
                .await?
                .into_iter()
                .find(|o| o.movie.year.is_some())
                .ok_or_else(|| format_err!("No show returned"))?
        };
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history");
        let year = movie_obj.movie.year.expect("No year");
        let data = hashmap! {
            "movies" => vec![
                WatchedMovieRequest {
                    watched_at: Utc::now(),
                    title: movie_obj.movie.title.clone(),
                    year,
                    ids: movie_obj.movie.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success".into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn remove_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let episode_obj = self.get_episode(imdb_id, season, episode).await?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history/remove");
        let data = hashmap! {
            "episodes" => vec![
                WatchedEpisodeRequest {
                    watched_at: Utc::now(),
                    ids: episode_obj.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success".into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn remove_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let movie_obj = self
            .get_movie_by_imdb_id(imdb_id)
            .await?
            .into_iter()
            .find(|o| o.movie.year.is_some())
            .ok_or_else(|| format_err!("No show returned"))?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history/remove");
        let year = movie_obj.movie.year.expect("No year");
        let data = hashmap! {
            "movies" => vec![
                WatchedMovieRequest {
                    watched_at: Utc::now(),
                    title: movie_obj.movie.title.clone(),
                    year,
                    ids: movie_obj.movie.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success".into(),
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct WatchedMovieRequest {
    #[serde(with = "iso_8601_datetime")]
    pub watched_at: DateTime<Utc>,
    pub title: StackString,
    pub year: i32,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug)]
struct WatchedEpisodeRequest {
    #[serde(with = "iso_8601_datetime")]
    pub watched_at: DateTime<Utc>,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug)]
struct AccessTokenResponse {
    access_token: StackString,
    token_type: StackString,
    expires_in: u64,
    refresh_token: StackString,
    scope: StackString,
    created_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraktIdObject {
    pub trakt: i32,
    pub imdb: Option<StackString>,
    pub slug: Option<StackString>,
    pub tvdb: Option<i32>,
    pub tmdb: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraktShowObject {
    pub title: StackString,
    pub year: Option<i32>,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktEpisodeObject {
    pub season: i32,
    pub number: i32,
    pub title: StackString,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WatchListShowsResponse {
    pub show: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktShowSearchResponse {
    pub show: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktMovieSearchResponse {
    pub movie: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedEpisode {
    pub number: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedSeason {
    pub number: i32,
    pub episodes: Vec<TraktWatchedEpisode>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedShowResponse {
    pub show: TraktShowObject,
    pub seasons: Vec<TraktWatchedSeason>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedMovieResponse {
    pub movie: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktCalendarResponse {
    #[serde(with = "iso_8601_datetime")]
    pub first_aired: DateTime<Utc>,
    pub episode: TraktEpisodeObject,
    pub show: TraktShowObject,
}

#[cfg(test)]
mod tests {
    use crate::{config::Config, trakt_connection::TraktConnection};
    use anyhow::Error;
    use stack_string::format_sstr;
    use std::fmt::Write;

    #[test]
    #[ignore]
    fn test_get_auth_url() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        let test_state = TraktConnection::get_random_string();
        let url = conn._get_auth_url(test_state.as_str())?;
        println!("url {}", url);
        let expected = format_sstr!(
            "https://trakt.tv/oauth/authorize?{a}{client_id}{b}{domain}%2Ftrakt%2Fcallback&state={state}",
            a="response_type=code&client_id=",
            client_id=conn.config.trakt_client_id,
            b="&redirect_uri=https%3A%2F%2F",
            domain=conn.config.domain,
            state=test_state,
        );
        assert_eq!(url.as_str(), expected.as_str());
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_read_auth_token() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        let auth_token = conn.read_auth_token().await?;
        assert_eq!(auth_token.scope, "public");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_watchlist_shows() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await;
        let result = conn.get_watchlist_shows().await?;
        assert!(result.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_show_by_imdb_id() -> Result<(), Error> {
        let imdb_id = "tt4270492";
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await;
        let result = conn.get_show_by_imdb_id(imdb_id).await?;
        assert_eq!(result[0].show.title, "Billions");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_watched_shows() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await;
        let result = conn.get_watched_shows().await?;
        assert!(result.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_watched_movies() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await;
        let result = conn.get_watched_movies().await?;
        println!("{}", result.len());
        assert!(result.len() > 5);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_calendar() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await;
        let result = conn.get_calendar().await?;
        println!("{}", result.len());
        assert!(result.len() > 1);
        Ok(())
    }
}
