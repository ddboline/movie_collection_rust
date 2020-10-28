use anyhow::Error;
use stack_string::StackString;

use movie_collection_lib::{
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{TraktActions, WatchListShow},
};

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
        match &self.action {
            TraktActions::Add => {
                trakt.init().await;
                if let Some(show) = trakt.get_watchlist_shows().await?.get(&self.imdb_url) {
                    show.insert_show(pool).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&&self.imdb_url, pool).await? {
                    show.delete_show(pool).await?;
                }
            }
            _ => {}
        }
        Ok(self.imdb_url)
    }
}
