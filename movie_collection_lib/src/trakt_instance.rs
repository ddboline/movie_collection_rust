use anyhow::{format_err, Error};
use cpython::{FromPyObject, PyResult, PyTuple, Python, PythonObject, ToPyObject};
use std::collections::HashMap;

use crate::{
    stack_string::StackString,
    trakt_utils::{TraktCalEntryList, TraktResult, WatchListShow, WatchedEpisode, WatchedMovie},
};

pub fn trakt_instance_call_noargs(py: Python, method: &str) -> PyResult<String> {
    let trakt_instance = py.import("trakt_instance.trakt_instance")?;
    let result = trakt_instance.call(py, method, PyTuple::empty(py), None)?;
    String::extract(py, &result)
}

pub fn trakt_instance_call_tuple(py: Python, method: &str, tup: PyTuple) -> PyResult<String> {
    let trakt_instance = py.import("trakt_instance.trakt_instance")?;
    let result = trakt_instance.call(py, method, tup, None)?;
    String::extract(py, &result)
}

pub fn get_watchlist_shows() -> Result<HashMap<StackString, WatchListShow>, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let result =
        trakt_instance_call_noargs(py, "get_watchlist").map_err(|e| format_err!("{:?}", e))?;
    let watchlist_shows: Vec<WatchListShow> = serde_json::from_str(&result)?;
    let watchlist_shows = watchlist_shows
        .into_iter()
        .map(|s| (s.link.clone(), s))
        .collect();
    Ok(watchlist_shows)
}

pub fn add_watchlist_show(imdb_id: &str) -> Result<TraktResult, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let tup = PyTuple::new(py, &[imdb_id.to_py_object(py).into_object()]);
    let result = trakt_instance_call_tuple(py, "add_to_watchlist", tup)
        .map_err(|e| format_err!("{:?}", e))?;
    let result: TraktResult = serde_json::from_str(&result)?;
    Ok(result)
}

pub fn remove_watchlist_show(imdb_id: &str) -> Result<TraktResult, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let tup = PyTuple::new(py, &[imdb_id.to_py_object(py).into_object()]);
    let result = trakt_instance_call_tuple(py, "delete_show_from_watchlist", tup)
        .map_err(|e| format_err!("{:?}", e))?;
    let result: TraktResult = serde_json::from_str(&result)?;
    Ok(result)
}

pub fn get_watched_shows() -> Result<HashMap<(StackString, i32, i32), WatchedEpisode>, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let result =
        trakt_instance_call_noargs(py, "get_watched_shows").map_err(|e| format_err!("{:?}", e))?;
    let watched_shows: Vec<WatchedEpisode> = serde_json::from_str(&result)?;
    let watched_shows: HashMap<(StackString, i32, i32), WatchedEpisode> = watched_shows
        .into_iter()
        .map(|s| ((s.imdb_url.clone(), s.season, s.episode), s))
        .collect();
    Ok(watched_shows)
}

pub fn get_watched_movies() -> Result<HashMap<StackString, WatchedMovie>, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let result =
        trakt_instance_call_noargs(py, "get_watched_movies").map_err(|e| format_err!("{:?}", e))?;
    let watched_movies: Vec<WatchedMovie> = serde_json::from_str(&result)?;
    let watched_movies: HashMap<StackString, WatchedMovie> = watched_movies
        .into_iter()
        .map(|s| (s.imdb_url.clone(), s))
        .collect();
    Ok(watched_movies)
}

pub fn get_calendar() -> Result<TraktCalEntryList, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let result =
        trakt_instance_call_noargs(py, "get_trakt_cal").map_err(|e| format_err!("{:?}", e))?;
    let calendar = serde_json::from_str(&result)?;
    Ok(calendar)
}

pub fn add_episode_to_watched(
    imdb_id: &str,
    season: i32,
    episode: i32,
) -> Result<TraktResult, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let tup = PyTuple::new(
        py,
        &[
            imdb_id.to_py_object(py).into_object(),
            season.to_string().to_py_object(py).into_object(),
            episode.to_string().to_py_object(py).into_object(),
        ],
    );
    let result = trakt_instance_call_tuple(py, "add_episode_to_watched", tup)
        .map_err(|e| format_err!("{:?}", e))?;
    let result: TraktResult = serde_json::from_str(&result)?;
    Ok(result)
}

pub fn add_movie_to_watched(imdb_id: &str) -> Result<TraktResult, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let tup = PyTuple::new(py, &[imdb_id.to_py_object(py).into_object()]);
    let result =
        trakt_instance_call_tuple(py, "add_to_watched", tup).map_err(|e| format_err!("{:?}", e))?;
    let result: TraktResult = serde_json::from_str(&result)?;
    Ok(result)
}

pub fn remove_episode_to_watched(
    imdb_id: &str,
    season: i32,
    episode: i32,
) -> Result<TraktResult, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let tup = PyTuple::new(
        py,
        &[
            imdb_id.to_py_object(py).into_object(),
            season.to_string().to_py_object(py).into_object(),
            episode.to_string().to_py_object(py).into_object(),
        ],
    );
    let result = trakt_instance_call_tuple(py, "delete_episode_from_watched", tup)
        .map_err(|e| format_err!("{:?}", e))?;
    let result: TraktResult = serde_json::from_str(&result)?;
    Ok(result)
}

pub fn remove_movie_to_watched(imdb_id: &str) -> Result<TraktResult, Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();
    let tup = PyTuple::new(py, &[imdb_id.to_py_object(py).into_object()]);
    let result = trakt_instance_call_tuple(py, "delete_movie_from_watched", tup)
        .map_err(|e| format_err!("{:?}", e))?;
    let result: TraktResult = serde_json::from_str(&result)?;
    Ok(result)
}
