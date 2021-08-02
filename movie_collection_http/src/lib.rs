#![allow(clippy::must_use_candidate)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::used_underscore_binding)]
#![allow(clippy::manual_map)]
#![allow(clippy::default_trait_access)]

pub mod datetime_wrapper;
pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;
pub mod naivedate_wrapper;
pub mod uuid_wrapper;

use rweb::Schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::str::FromStr;

use movie_collection_lib::{
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{LastModifiedResponse, MovieCollectionRow},
    movie_queue::MovieQueueRow,
    plex_events::{PlexEvent, PlexEventType, PlexFilename},
    trakt_utils::TraktActions,
    tv_show_source::TvShowSource,
};

use crate::{datetime_wrapper::DateTimeWrapper, naivedate_wrapper::NaiveDateWrapper};

#[derive(Clone, Serialize, Deserialize, Schema)]
pub struct ImdbEpisodesWrapper {
    pub show: StackString,
    pub title: StackString,
    pub season: i32,
    pub episode: i32,
    pub airdate: NaiveDateWrapper,
    pub rating: f64,
    pub eptitle: StackString,
    pub epurl: StackString,
}

impl From<ImdbEpisodes> for ImdbEpisodesWrapper {
    fn from(item: ImdbEpisodes) -> Self {
        Self {
            show: item.show,
            title: item.title,
            season: item.season,
            episode: item.episode,
            airdate: item.airdate.into(),
            rating: item.rating,
            eptitle: item.eptitle,
            epurl: item.epurl,
        }
    }
}

impl From<ImdbEpisodesWrapper> for ImdbEpisodes {
    fn from(item: ImdbEpisodesWrapper) -> Self {
        Self {
            show: item.show,
            title: item.title,
            season: item.season,
            episode: item.episode,
            airdate: item.airdate.into(),
            rating: item.rating,
            eptitle: item.eptitle,
            epurl: item.epurl,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, Copy, PartialEq, Schema)]
pub enum TvShowSourceWrapper {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "amazon")]
    Amazon,
    #[serde(rename = "hulu")]
    Hulu,
    #[serde(rename = "netflix")]
    Netflix,
}

impl From<TvShowSource> for TvShowSourceWrapper {
    fn from(item: TvShowSource) -> Self {
        match item {
            TvShowSource::All => Self::All,
            TvShowSource::Amazon => Self::Amazon,
            TvShowSource::Hulu => Self::Hulu,
            TvShowSource::Netflix => Self::Netflix,
        }
    }
}

impl From<TvShowSourceWrapper> for TvShowSource {
    fn from(item: TvShowSourceWrapper) -> Self {
        match item {
            TvShowSourceWrapper::All => Self::All,
            TvShowSourceWrapper::Amazon => Self::Amazon,
            TvShowSourceWrapper::Hulu => Self::Hulu,
            TvShowSourceWrapper::Netflix => Self::Netflix,
        }
    }
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, Schema)]
pub struct ImdbRatingsWrapper {
    pub index: i32,
    pub show: StackString,
    pub title: Option<StackString>,
    pub link: StackString,
    pub rating: Option<f64>,
    pub istv: Option<bool>,
    pub source: Option<TvShowSourceWrapper>,
}

impl From<ImdbRatings> for ImdbRatingsWrapper {
    fn from(item: ImdbRatings) -> Self {
        Self {
            index: item.index,
            show: item.show,
            title: item.title,
            link: item.link,
            rating: item.rating,
            istv: item.istv,
            source: item.source.map(Into::into),
        }
    }
}

impl From<ImdbRatingsWrapper> for ImdbRatings {
    fn from(item: ImdbRatingsWrapper) -> Self {
        Self {
            index: item.index,
            show: item.show,
            title: item.title,
            link: item.link,
            rating: item.rating,
            istv: item.istv,
            source: item.source.map(Into::into),
        }
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Schema)]
pub struct MovieQueueRowWrapper {
    pub idx: i32,
    pub collection_idx: i32,
    pub path: StackString,
    pub show: StackString,
    pub last_modified: Option<DateTimeWrapper>,
}

impl From<MovieQueueRow> for MovieQueueRowWrapper {
    fn from(item: MovieQueueRow) -> Self {
        Self {
            idx: item.idx,
            collection_idx: item.collection_idx,
            path: item.path,
            show: item.show,
            last_modified: item.last_modified.map(Into::into),
        }
    }
}

#[derive(Default, Serialize, Deserialize, Schema)]
pub struct MovieCollectionRowWrapper {
    pub idx: i32,
    pub path: StackString,
    pub show: StackString,
}

impl From<MovieCollectionRow> for MovieCollectionRowWrapper {
    fn from(item: MovieCollectionRow) -> Self {
        Self {
            idx: item.idx,
            path: item.path,
            show: item.show,
        }
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct LastModifiedResponseWrapper {
    pub table: StackString,
    pub last_modified: DateTimeWrapper,
}

impl From<LastModifiedResponse> for LastModifiedResponseWrapper {
    fn from(item: LastModifiedResponse) -> Self {
        Self {
            table: item.table,
            last_modified: item.last_modified.into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct PlexEventWrapper {
    pub event: StackString,
    pub account: StackString,
    pub server: StackString,
    pub player_title: StackString,
    pub player_address: StackString,
    pub title: StackString,
    pub parent_title: Option<StackString>,
    pub grandparent_title: Option<StackString>,
    pub added_at: DateTimeWrapper,
    pub updated_at: Option<DateTimeWrapper>,
    pub last_modified: DateTimeWrapper,
    pub metadata_type: Option<StackString>,
    pub section_type: Option<StackString>,
    pub section_title: Option<StackString>,
    pub metadata_key: Option<StackString>,
}

impl From<PlexEvent> for PlexEventWrapper {
    fn from(item: PlexEvent) -> Self {
        Self {
            event: item.event,
            account: item.account,
            server: item.server,
            player_title: item.player_title,
            player_address: item.player_address,
            title: item.title,
            parent_title: item.parent_title,
            grandparent_title: item.grandparent_title,
            added_at: item.added_at.into(),
            updated_at: item.updated_at.map(Into::into),
            last_modified: item.last_modified.into(),
            metadata_type: item.metadata_type,
            section_type: item.section_type,
            section_title: item.section_title,
            metadata_key: item.metadata_key,
        }
    }
}

impl From<PlexEventWrapper> for PlexEvent {
    fn from(item: PlexEventWrapper) -> Self {
        Self {
            event: item.event,
            account: item.account,
            server: item.server,
            player_title: item.player_title,
            player_address: item.player_address,
            title: item.title,
            parent_title: item.parent_title,
            grandparent_title: item.grandparent_title,
            added_at: item.added_at.into(),
            updated_at: item.updated_at.map(Into::into),
            last_modified: item.last_modified.into(),
            metadata_type: item.metadata_type,
            section_type: item.section_type,
            section_title: item.section_title,
            metadata_key: item.metadata_key,
        }
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Schema)]
pub struct PlexFilenameWrapper {
    pub metadata_key: StackString,
    pub filename: StackString,
}

impl From<PlexFilename> for PlexFilenameWrapper {
    fn from(item: PlexFilename) -> Self {
        Self {
            metadata_key: item.metadata_key,
            filename: item.filename,
        }
    }
}

impl From<PlexFilenameWrapper> for PlexFilename {
    fn from(item: PlexFilenameWrapper) -> Self {
        Self {
            metadata_key: item.metadata_key,
            filename: item.filename,
        }
    }
}

#[derive(Clone, Copy, Schema)]
pub enum TraktActionsWrapper {
    None,
    List,
    Add,
    Remove,
}

impl From<TraktActions> for TraktActionsWrapper {
    fn from(item: TraktActions) -> Self {
        match item {
            TraktActions::None => Self::None,
            TraktActions::List => Self::List,
            TraktActions::Add => Self::Add,
            TraktActions::Remove => Self::Remove,
        }
    }
}

impl From<TraktActionsWrapper> for TraktActions {
    fn from(item: TraktActionsWrapper) -> Self {
        match item {
            TraktActionsWrapper::None => Self::None,
            TraktActionsWrapper::List => Self::List,
            TraktActionsWrapper::Add => Self::Add,
            TraktActionsWrapper::Remove => Self::Remove,
        }
    }
}

impl FromStr for TraktActionsWrapper {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(TraktActions::from(s).into())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Schema)]
pub enum PlexEventTypeWrapper {
    LibraryOnDeck,
    LibraryNew,
    MediaPause,
    MediaPlay,
    MediaRate,
    MediaResume,
    MediaScrobble,
    MediaStop,
    AdminDatabaseBackup,
    AdminDatabaseCorrupted,
    DeviceNew,
    PlaybackStarted,
}

impl From<PlexEventType> for PlexEventTypeWrapper {
    fn from(item: PlexEventType) -> Self {
        match item {
            PlexEventType::LibraryOnDeck => Self::LibraryOnDeck,
            PlexEventType::LibraryNew => Self::LibraryNew,
            PlexEventType::MediaPause => Self::MediaPause,
            PlexEventType::MediaPlay => Self::MediaPlay,
            PlexEventType::MediaRate => Self::MediaRate,
            PlexEventType::MediaResume => Self::MediaResume,
            PlexEventType::MediaScrobble => Self::MediaScrobble,
            PlexEventType::MediaStop => Self::MediaStop,
            PlexEventType::AdminDatabaseBackup => Self::AdminDatabaseBackup,
            PlexEventType::AdminDatabaseCorrupted => Self::AdminDatabaseCorrupted,
            PlexEventType::DeviceNew => Self::DeviceNew,
            PlexEventType::PlaybackStarted => Self::PlaybackStarted,
        }
    }
}

impl From<PlexEventTypeWrapper> for PlexEventType {
    fn from(item: PlexEventTypeWrapper) -> Self {
        match item {
            PlexEventTypeWrapper::LibraryOnDeck => Self::LibraryOnDeck,
            PlexEventTypeWrapper::LibraryNew => Self::LibraryNew,
            PlexEventTypeWrapper::MediaPause => Self::MediaPause,
            PlexEventTypeWrapper::MediaPlay => Self::MediaPlay,
            PlexEventTypeWrapper::MediaRate => Self::MediaRate,
            PlexEventTypeWrapper::MediaResume => Self::MediaResume,
            PlexEventTypeWrapper::MediaScrobble => Self::MediaScrobble,
            PlexEventTypeWrapper::MediaStop => Self::MediaStop,
            PlexEventTypeWrapper::AdminDatabaseBackup => Self::AdminDatabaseBackup,
            PlexEventTypeWrapper::AdminDatabaseCorrupted => Self::AdminDatabaseCorrupted,
            PlexEventTypeWrapper::DeviceNew => Self::DeviceNew,
            PlexEventTypeWrapper::PlaybackStarted => Self::PlaybackStarted,
        }
    }
}
