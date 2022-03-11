use async_graphql::{
    dataloader::{DataLoader, Loader},
    Context, Enum, Error, Object,
};
use async_trait::async_trait;
use stack_string::StackString;
use std::collections::HashMap;

use movie_collection_lib::{
    imdb_ratings::ImdbRatings, pgpool::PgPool, tv_show_source::TvShowSource,
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

pub struct ImdbItem {
    pub show: String,
    pub title: Option<String>,
    pub link: String,
    pub rating: Option<f64>,
    pub item_type: Option<ImdbType>,
    pub source: Option<ImdbTvSource>,
}

impl From<ImdbRatings> for ImdbItem {
    fn from(item: ImdbRatings) -> Self {
        Self {
            show: item.show.into(),
            title: item.title.map(Into::into),
            link: item.link.into(),
            rating: item.rating,
            item_type: item.istv.map(Into::into),
            source: item.source.map(Into::into),
        }
    }
}

#[Object]
impl ImdbItem {
    /// Imdb Show
    async fn show(&self) -> &str {
        &self.show
    }

    /// Imdb Show Title
    async fn title(&self) -> Option<&str> {
        self.title.as_ref().map(String::as_str)
    }

    /// Imdb Rating
    async fn rating(&self) -> Option<f64> {
        self.rating
    }

    /// Imdb Type
    async fn imdb_type(&self) -> Option<ImdbType> {
        self.item_type
    }

    /// Imdb Source
    async fn imdb_source(&self) -> Option<ImdbTvSource> {
        self.source
    }
}

pub struct ItemLoader(PgPool);

impl ItemLoader {
    pub fn new(pool: PgPool) -> Self {
        Self(pool)
    }
}

#[async_trait]
impl Loader<StackString> for ItemLoader {
    type Value = ImdbRatings;
    type Error = Error;

    async fn load(
        &self,
        keys: &[StackString],
    ) -> Result<HashMap<StackString, Self::Value>, Self::Error> {
        let mut shows = HashMap::new();
        for key in keys {
            if let Some(item) = ImdbRatings::get_show_by_link(&key, &self.0)
                .await
                .map_err(|e| Error::new_with_source(e))?
            {
                shows.insert(key.clone(), item);
            }
        }
        Ok(shows)
    }
}

pub struct QueryRoot;

#[Object]
impl<'a> QueryRoot {
    async fn rating(&self, ctx: &Context<'a>, show: String) -> Result<Option<ImdbItem>, Error> {
        let show = ctx
            .data::<DataLoader<ItemLoader>>()?
            .load_one(show.as_str().into())
            .await?;
        let show = show.map(Into::into);
        Ok(show)
    }
}
