use cpython::{FromPyObject, PyResult, PyTuple, Python};
use failure::{err_msg, Error};
use std::collections::HashMap;

use crate::common::config::Config;
use crate::common::trakt_utils::{
    TraktCalEntryList, TraktConnectionTrait, TraktResult, WatchListShow, WatchedEpisode,
    WatchedMovie,
};

pub struct TraktInstance {
    pub config: Config,
}

impl TraktInstance {
    pub fn new() -> Self {
        Self {
            config: Config::with_config().expect("Config init failed"),
        }
    }

    pub fn get_watchlist(&self, py: Python) -> PyResult<String> {
        let trakt_instance = py.import("trakt_instance.trakt_instance")?;
        let result = trakt_instance.call(py, "get_watchlist", PyTuple::empty(py), None)?;
        String::extract(py, &result)
    }
}

impl TraktConnectionTrait for TraktInstance {
    fn get_watchlist_shows(&self) -> Result<HashMap<String, WatchListShow>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let result = self
            .get_watchlist(py)
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let watchlist_shows: Vec<WatchListShow> = serde_json::from_str(&result)?;
        let watchlist_shows = watchlist_shows
            .into_iter()
            .map(|s| (s.link.clone(), s))
            .collect();
        Ok(watchlist_shows)
    }

    fn add_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        Ok(Default::default())
    }

    fn remove_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        Ok(Default::default())
    }

    fn get_watched_shows(&self) -> Result<HashMap<(String, i32, i32), WatchedEpisode>, Error> {
        Ok(HashMap::new())
    }

    fn get_watched_movies(&self) -> Result<HashMap<String, WatchedMovie>, Error> {
        Ok(HashMap::new())
    }

    fn get_calendar(&self) -> Result<TraktCalEntryList, Error> {
        Ok(Vec::new())
    }

    fn add_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        Ok(Default::default())
    }

    fn add_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        Ok(Default::default())
    }

    fn remove_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        Ok(Default::default())
    }

    fn remove_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        Ok(Default::default())
    }
}
