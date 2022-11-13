use async_graphql::{
    dataloader::{DataLoader, Loader},
    ComplexObject, Context, Enum, Error, Object, SimpleObject,
};
use async_trait::async_trait;
use derive_more::{Deref, From, Into};
use futures::TryStreamExt;
use stack_string::StackString;
use std::collections::HashMap;
use time::Date;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use movie_collection_lib::{
    imdb_episodes::{ImdbEpisodes, ImdbSeason},
    imdb_ratings::ImdbRatings,
    pgpool::PgPool,
    tv_show_source::TvShowSource,
};

#[derive(Copy, Clone, Enum, PartialEq, Eq)]
pub enum ImdbType {
    /// Television Show
    Television,
    /// Movie
    Movie,
}

impl ImdbType {
    fn is_tv(self) -> bool {
        match self {
            Self::Television => true,
            Self::Movie => false,
        }
    }
}

impl From<bool> for ImdbType {
    fn from(item: bool) -> Self {
        if item {
            Self::Television
        } else {
            Self::Movie
        }
    }
}

impl From<ImdbType> for bool {
    fn from(item: ImdbType) -> bool {
        item.is_tv()
    }
}

#[derive(Copy, Clone, Enum, PartialEq, Eq)]
pub enum ImdbTvSource {
    All,
    Amazon,
    Hulu,
    Netflix,
}

impl From<TvShowSource> for ImdbTvSource {
    fn from(item: TvShowSource) -> Self {
        match item {
            TvShowSource::All => Self::All,
            TvShowSource::Amazon => Self::Amazon,
            TvShowSource::Hulu => Self::Hulu,
            TvShowSource::Netflix => Self::Netflix,
        }
    }
}

impl From<ImdbTvSource> for TvShowSource {
    fn from(item: ImdbTvSource) -> Self {
        match item {
            ImdbTvSource::All => Self::All,
            ImdbTvSource::Amazon => Self::Amazon,
            ImdbTvSource::Hulu => Self::Hulu,
            ImdbTvSource::Netflix => Self::Netflix,
        }
    }
}

#[derive(SimpleObject)]
#[graphql(complex)]
pub struct ImdbItem {
    /// Imdb Show
    pub show: StackString,
    /// Imdb Show Title
    pub title: Option<StackString>,
    /// Imdb Show Link
    pub link: StackString,
    /// Imdb Rating
    pub rating: Option<f64>,
    /// Imdb Show Type
    pub item_type: Option<ImdbType>,
    /// Imdb Show Source
    pub source: Option<ImdbTvSource>,
}

impl From<ImdbRatings> for ImdbItem {
    fn from(item: ImdbRatings) -> Self {
        Self {
            show: item.show,
            title: item.title,
            link: item.link,
            rating: item.rating,
            item_type: item.istv.map(Into::into),
            source: item.source.map(Into::into),
        }
    }
}

#[ComplexObject]
impl ImdbItem {
    /// Show seasons
    async fn seasons(&self, ctx: &Context<'_>) -> Result<Vec<SeasonItem>, Error> {
        let key = ShowSeasonKey(self.show.clone());
        let values = ctx
            .data::<DataLoader<ItemLoader>>()?
            .load_one(key)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(values)
    }

    /// Show episodes
    async fn episodes(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Season")] season: Option<i32>,
    ) -> Result<Vec<EpisodeItem>, Error> {
        let key = EpisodesShowSeasonKey {
            show: self.show.clone(),
            season,
            episode: None,
        };
        let values = ctx
            .data::<DataLoader<ItemLoader>>()?
            .load_one(key)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(values)
    }
}

#[derive(SimpleObject)]
#[graphql(complex)]
pub struct SeasonItem {
    pub show: StackString,
    pub title: StackString,
    pub season: i32,
    pub nepisodes: i64,
}

#[ComplexObject]
impl SeasonItem {
    /// Imdb Show
    async fn imdb_show(&self, ctx: &Context<'_>) -> Result<ImdbItem, Error> {
        let key = RatingsShowKey(self.show.clone());
        let value = ctx
            .data::<DataLoader<ItemLoader>>()?
            .load_one(key)
            .await?
            .ok_or_else(|| Error::new("Show not found"))?
            .into();
        Ok(value)
    }

    /// Season Episodes
    async fn episodes(&self, ctx: &Context<'_>) -> Result<Vec<EpisodeItem>, Error> {
        let key = EpisodesShowSeasonKey {
            show: self.show.clone(),
            season: Some(self.season),
            episode: None,
        };
        let values = ctx
            .data::<DataLoader<ItemLoader>>()?
            .load_one(key)
            .await?
            .ok_or_else(|| Error::new("Show not found"))?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(values)
    }
}

impl From<ImdbSeason> for SeasonItem {
    fn from(item: ImdbSeason) -> Self {
        Self {
            show: item.show,
            title: item.title,
            season: item.season,
            nepisodes: item.nepisodes,
        }
    }
}

impl From<SeasonItem> for ImdbSeason {
    fn from(item: SeasonItem) -> Self {
        Self {
            show: item.show,
            title: item.title,
            season: item.season,
            nepisodes: item.nepisodes,
        }
    }
}

#[derive(SimpleObject)]
pub struct EpisodeItem {
    /// Imdb Show
    pub show: StackString,
    /// Show Title
    pub title: StackString,
    /// Season
    pub season: i32,
    /// Episode
    pub episode: i32,
    /// Airdate
    pub airdate: Option<Date>,
    /// Imdb Episode Rating
    pub rating: Option<f64>,
    /// Episode Title
    pub eptitle: StackString,
    /// Episode Imdb Link
    pub epurl: StackString,
}

impl From<ImdbEpisodes> for EpisodeItem {
    fn from(item: ImdbEpisodes) -> Self {
        Self {
            show: item.show,
            title: item.title,
            season: item.season,
            episode: item.episode,
            airdate: item.airdate,
            rating: item.rating.and_then(|r| r.to_f64()),
            eptitle: item.eptitle,
            epurl: item.epurl,
        }
    }
}

impl From<EpisodeItem> for ImdbEpisodes {
    fn from(item: EpisodeItem) -> Self {
        Self {
            show: item.show,
            title: item.title,
            season: item.season,
            episode: item.episode,
            airdate: item.airdate,
            rating: item.rating.and_then(|r| Decimal::from_f64_retain(r)),
            eptitle: item.eptitle,
            epurl: item.epurl,
        }
    }
}

pub struct ItemLoader(PgPool);

impl ItemLoader {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self(pool)
    }
}

#[derive(From, Into, Clone, PartialEq, Eq, Hash, Deref)]
pub struct RatingsShowKey(StackString);

impl From<&str> for RatingsShowKey {
    fn from(s: &str) -> Self {
        let s: StackString = s.into();
        Self(s)
    }
}

#[async_trait]
impl Loader<RatingsShowKey> for ItemLoader {
    type Value = ImdbRatings;
    type Error = Error;

    async fn load(
        &self,
        keys: &[RatingsShowKey],
    ) -> Result<HashMap<RatingsShowKey, Self::Value>, Self::Error> {
        let mut shows = HashMap::new();
        for key in keys {
            if let Some(item) = ImdbRatings::get_show_by_link(key.as_str(), &self.0)
                .await
                .map_err(Error::new_with_source)?
            {
                shows.insert(key.clone(), item);
            }
        }
        Ok(shows)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, From, Into, Deref)]
pub struct ShowSeasonKey(StackString);

#[async_trait]
impl Loader<ShowSeasonKey> for ItemLoader {
    type Value = Vec<ImdbSeason>;
    type Error = Error;

    async fn load(
        &self,
        keys: &[ShowSeasonKey],
    ) -> Result<HashMap<ShowSeasonKey, Self::Value>, Self::Error> {
        let mut episodes = HashMap::new();
        for key in keys {
            let show = key.as_str();
            let values: Vec<_> = ImdbSeason::get_seasons(show, &self.0)
                .await
                .map_err(Error::new_with_source)?
                .try_collect()
                .await?;
            episodes.insert(key.clone(), values);
        }
        Ok(episodes)
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct EpisodesShowSeasonKey {
    show: StackString,
    season: Option<i32>,
    episode: Option<i32>,
}

#[async_trait]
impl Loader<EpisodesShowSeasonKey> for ItemLoader {
    type Value = Vec<ImdbEpisodes>;
    type Error = Error;

    async fn load(
        &self,
        keys: &[EpisodesShowSeasonKey],
    ) -> Result<HashMap<EpisodesShowSeasonKey, Self::Value>, Self::Error> {
        let mut episodes = HashMap::new();
        for key in keys {
            let EpisodesShowSeasonKey {
                show,
                season,
                episode,
            } = key;
            let values: Vec<_> =
                ImdbEpisodes::get_episodes_by_show_season_episode(show, *season, *episode, &self.0)
                    .await
                    .map_err(Error::new_with_source)?
                    .try_collect()
                    .await?;
            episodes.insert(key.clone(), values);
        }
        Ok(episodes)
    }
}

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn imdb_show(
        &self,
        ctx: &Context<'_>,
        show: StackString,
    ) -> Result<Option<ImdbItem>, Error> {
        let key: RatingsShowKey = show.as_str().into();
        let show = ctx.data::<DataLoader<ItemLoader>>()?.load_one(key).await?;
        let show = show.map(Into::into);
        Ok(show)
    }

    async fn episodes(
        &self,
        ctx: &Context<'_>,
        show: StackString,
        season: Option<i32>,
    ) -> Result<Vec<EpisodeItem>, Error> {
        let key = EpisodesShowSeasonKey {
            show,
            season,
            episode: None,
        };
        let episodes = ctx
            .data::<DataLoader<ItemLoader>>()?
            .load_one(key)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(episodes)
    }
}
