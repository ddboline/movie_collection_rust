use cpython::{FromPyObject, PyResult, PyString, PyTuple, Python, PythonObject};
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

    pub fn trakt_instance_call_noargs(&self, py: Python, method: &str) -> PyResult<String> {
        let trakt_instance = py.import("trakt_instance.trakt_instance")?;
        let result = trakt_instance.call(py, method, PyTuple::empty(py), None)?;
        String::extract(py, &result)
    }

    pub fn trakt_instance_call_tuple(
        &self,
        py: Python,
        method: &str,
        tup: PyTuple,
    ) -> PyResult<String> {
        let trakt_instance = py.import("trakt_instance.trakt_instance")?;
        let result = trakt_instance.call(py, method, tup, None)?;
        String::extract(py, &result)
    }
}

impl TraktConnectionTrait for TraktInstance {
    fn get_watchlist_shows(&self) -> Result<HashMap<String, WatchListShow>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let result = self
            .trakt_instance_call_noargs(py, "get_watchlist")
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let watchlist_shows: Vec<WatchListShow> = serde_json::from_str(&result)?;
        let watchlist_shows = watchlist_shows
            .into_iter()
            .map(|s| (s.link.clone(), s))
            .collect();
        Ok(watchlist_shows)
    }

    fn add_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let tup = PyTuple::new(py, &[PyString::new(py, imdb_id).into_object()]);
        let result = self
            .trakt_instance_call_tuple(py, "add_to_watchlist", tup)
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let result: TraktResult = serde_json::from_str(&result)?;
        Ok(result)
    }

    fn remove_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let tup = PyTuple::new(py, &[PyString::new(py, imdb_id).into_object()]);
        let result = self
            .trakt_instance_call_tuple(py, "delete_show_from_watchlist", tup)
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let result: TraktResult = serde_json::from_str(&result)?;
        Ok(result)
    }

    fn get_watched_shows(&self) -> Result<HashMap<(String, i32, i32), WatchedEpisode>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let result = self
            .trakt_instance_call_noargs(py, "get_watched_shows")
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let watched_shows: Vec<WatchedEpisode> = serde_json::from_str(&result)?;
        let watched_shows: HashMap<(String, i32, i32), WatchedEpisode> = watched_shows
            .into_iter()
            .map(|s| ((s.imdb_url.clone(), s.season, s.episode), s))
            .collect();
        Ok(watched_shows)
    }

    fn get_watched_movies(&self) -> Result<HashMap<String, WatchedMovie>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let result = self
            .trakt_instance_call_noargs(py, "get_watched_movies")
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let watched_movies: Vec<WatchedMovie> = serde_json::from_str(&result)?;
        let watched_movies: HashMap<String, WatchedMovie> = watched_movies
            .into_iter()
            .map(|s| (s.imdb_url.clone(), s))
            .collect();
        Ok(watched_movies)
    }

    fn get_calendar(&self) -> Result<TraktCalEntryList, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let result = self
            .trakt_instance_call_noargs(py, "get_trakt_cal")
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let calendar = serde_json::from_str(&result)?;
        Ok(calendar)
    }

    fn add_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let tup = PyTuple::new(
            py,
            &[
                PyString::new(py, imdb_id).into_object(),
                PyString::new(py, &season.to_string()).into_object(),
                PyString::new(py, &episode.to_string()).into_object(),
            ],
        );
        let result = self
            .trakt_instance_call_tuple(py, "add_episode_to_watched", tup)
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let result: TraktResult = serde_json::from_str(&result)?;
        Ok(result)
    }

    fn add_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let tup = PyTuple::new(py, &[PyString::new(py, imdb_id).into_object()]);
        let result = self
            .trakt_instance_call_tuple(py, "add_to_watched", tup)
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let result: TraktResult = serde_json::from_str(&result)?;
        Ok(result)
    }

    fn remove_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let tup = PyTuple::new(
            py,
            &[
                PyString::new(py, imdb_id).into_object(),
                PyString::new(py, &season.to_string()).into_object(),
                PyString::new(py, &episode.to_string()).into_object(),
            ],
        );
        let result = self
            .trakt_instance_call_tuple(py, "delete_episode_from_watched", tup)
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let result: TraktResult = serde_json::from_str(&result)?;
        Ok(result)
    }

    fn remove_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let tup = PyTuple::new(py, &[PyString::new(py, imdb_id).into_object()]);
        let result = self
            .trakt_instance_call_tuple(py, "delete_movie_from_watched", tup)
            .map_err(|e| err_msg(format!("{:?}", e)))?;
        let result: TraktResult = serde_json::from_str(&result)?;
        Ok(result)
    }
}
