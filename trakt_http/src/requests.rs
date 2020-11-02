use anyhow::{Error};
use stack_string::StackString;
use stdout_channel::{MockStdout, StdoutChannel};

use movie_collection_lib::{
    movie_collection::{
        ImdbSeason,  MovieCollection,
    },
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{
        watch_list_http_worker,
        watched_action_http_worker, TraktActions, WatchListShow, 
    },
};

use super::app::CONFIG;

pub struct ImdbSeasonsRequest {
    pub show: StackString,
}

impl ImdbSeasonsRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<ImdbSeason>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        if &self.show == "" {
            Ok(Vec::new())
        } else {
            MovieCollection::new(&CONFIG, pool, &stdout)
                .print_imdb_all_seasons(&self.show)
                .await
        }
    }
}

pub struct WatchlistActionRequest {
    pub action: TraktActions,
    pub imdb_url: StackString,
}

impl WatchlistActionRequest {
    pub async fn handle(
        self,
        pool: &PgPool,
        trakt: &TraktConnection,
    ) -> Result<StackString, Error> {
        match self.action {
            TraktActions::Add => {
                trakt.init().await;
                if let Some(show) = trakt.get_watchlist_shows().await?.get(&self.imdb_url) {
                    show.insert_show(&pool).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&self.imdb_url, &pool).await? {
                    show.delete_show(&pool).await?;
                }
            }
            _ => {}
        }
        Ok(self.imdb_url)
    }
}

pub struct WatchedListRequest {
    pub imdb_url: StackString,
    pub season: i32,
}

impl WatchedListRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<StackString, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        watch_list_http_worker(&CONFIG, pool, &stdout, &self.imdb_url, self.season).await
    }
}

pub struct WatchedActionRequest {
    pub action: TraktActions,
    pub imdb_url: StackString,
    pub season: i32,
    pub episode: i32,
}

impl WatchedActionRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        trakt: &TraktConnection,
    ) -> Result<StackString, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        watched_action_http_worker(
            trakt,
            pool,
            self.action,
            &self.imdb_url,
            self.season,
            self.episode,
            &CONFIG,
            &stdout,
        )
        .await
    }
}
