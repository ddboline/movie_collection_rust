use anyhow::Error;
use stack_string::StackString;
use stdout_channel::{MockStdout, StdoutChannel};

use movie_collection_lib::{
    movie_collection::{ImdbSeason, MovieCollection},
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{TraktActions, WatchListShow},
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
