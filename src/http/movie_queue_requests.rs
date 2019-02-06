use actix::{Handler, Message};
use failure::Error;
use std::path;

use crate::common::imdb_ratings::ImdbRatings;
use crate::common::movie_collection::{
    ImdbSeason, MovieCollection, MovieCollectionDB, TvShowsResult,
};
use crate::common::movie_queue::{MovieQueueDB, MovieQueueResult};
use crate::common::pgpool::PgPool;
use crate::common::trakt_utils::{get_watchlist_shows_db_map, WatchListMap};

pub struct TvShowsRequest {}

impl Message for TvShowsRequest {
    type Result = Result<Vec<TvShowsResult>, Error>;
}

impl Handler<TvShowsRequest> for PgPool {
    type Result = Result<Vec<TvShowsResult>, Error>;

    fn handle(&mut self, _: TvShowsRequest, _: &mut Self::Context) -> Self::Result {
        MovieCollectionDB::with_pool(&self).print_tv_shows()
    }
}

pub struct WatchlistShowsRequest {}

impl Message for WatchlistShowsRequest {
    type Result = Result<WatchListMap, Error>;
}

impl Handler<WatchlistShowsRequest> for PgPool {
    type Result = Result<WatchListMap, Error>;

    fn handle(&mut self, _: WatchlistShowsRequest, _: &mut Self::Context) -> Self::Result {
        get_watchlist_shows_db_map(&self)
    }
}

pub struct QueueDeleteRequest {
    pub path: String,
}

impl Message for QueueDeleteRequest {
    type Result = Result<(), Error>;
}

impl Handler<QueueDeleteRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(&mut self, msg: QueueDeleteRequest, _: &mut Self::Context) -> Self::Result {
        if path::Path::new(&msg.path).exists() {
            MovieQueueDB::with_pool(&self).remove_from_queue_by_path(&msg.path)
        } else {
            Ok(())
        }
    }
}

pub struct MovieQueueRequest {
    pub patterns: Vec<String>,
}

impl Message for MovieQueueRequest {
    type Result = Result<Vec<MovieQueueResult>, Error>;
}

impl Handler<MovieQueueRequest> for PgPool {
    type Result = Result<Vec<MovieQueueResult>, Error>;

    fn handle(&mut self, msg: MovieQueueRequest, _: &mut Self::Context) -> Self::Result {
        MovieQueueDB::with_pool(&self).print_movie_queue(&msg.patterns)
    }
}

pub struct MoviePathRequest {
    pub idx: i32,
}

impl Message for MoviePathRequest {
    type Result = Result<String, Error>;
}

impl Handler<MoviePathRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&mut self, msg: MoviePathRequest, _: &mut Self::Context) -> Self::Result {
        MovieCollectionDB::with_pool(&self).get_collection_path(msg.idx)
    }
}

pub struct ImdbRatingsRequest {
    pub imdb_url: String,
}

impl Message for ImdbRatingsRequest {
    type Result = Result<Option<ImdbRatings>, Error>;
}

impl Handler<ImdbRatingsRequest> for PgPool {
    type Result = Result<Option<ImdbRatings>, Error>;

    fn handle(&mut self, msg: ImdbRatingsRequest, _: &mut Self::Context) -> Self::Result {
        ImdbRatings::get_show_by_link(&msg.imdb_url, &self)
    }
}

pub struct ImdbSeasonsRequest {
    pub show: String,
}

impl Message for ImdbSeasonsRequest {
    type Result = Result<Vec<ImdbSeason>, Error>;
}

impl Handler<ImdbSeasonsRequest> for PgPool {
    type Result = Result<Vec<ImdbSeason>, Error>;

    fn handle(&mut self, msg: ImdbSeasonsRequest, _: &mut Self::Context) -> Self::Result {
        if msg.show.as_str() == "" {
            Ok(Vec::new())
        } else {
            MovieCollectionDB::with_pool(&self).print_imdb_all_seasons(&msg.show)
        }
    }
}
