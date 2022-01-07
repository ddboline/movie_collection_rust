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
#![allow(clippy::unused_async)]
#![allow(clippy::missing_panics_doc)]

pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;

use chrono::{DateTime, NaiveDate, Utc};
use derive_more::{Deref, Display, From, FromStr, Into};
use rweb::Schema;
use rweb_helper::derive_rweb_schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use movie_collection_lib::{
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{LastModifiedResponse, MovieCollectionRow},
    movie_queue::MovieQueueRow,
    plex_events::{PlexEvent, PlexEventType, PlexFilename},
    trakt_utils::TraktActions,
    tv_show_source::TvShowSource,
};

#[derive(Clone, Serialize, Deserialize, Into, From)]
pub struct ImdbEpisodesWrapper(ImdbEpisodes);

derive_rweb_schema!(ImdbEpisodesWrapper, _ImdbEpisodesWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _ImdbEpisodesWrapper {
    #[schema(description = "TV Show Name")]
    show: StackString,
    #[schema(description = "Title")]
    title: StackString,
    #[schema(description = "Season")]
    season: i32,
    #[schema(description = "Episode")]
    episode: i32,
    #[schema(description = "Airdate")]
    airdate: NaiveDate,
    #[schema(description = "Rating")]
    rating: f64,
    #[schema(description = "Episode Title")]
    eptitle: StackString,
    #[schema(description = "Episode URL")]
    epurl: StackString,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, Copy, PartialEq, Into, From)]
pub struct TvShowSourceWrapper(TvShowSource);

derive_rweb_schema!(TvShowSourceWrapper, _TvShowSourceWrapper);

#[allow(dead_code)]
#[derive(Serialize, Schema)]
enum _TvShowSourceWrapper {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "amazon")]
    Amazon,
    #[serde(rename = "hulu")]
    Hulu,
    #[serde(rename = "netflix")]
    Netflix,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, Into, From)]
pub struct ImdbRatingsWrapper(ImdbRatings);

derive_rweb_schema!(ImdbRatingsWrapper, _ImdbRatingsWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _ImdbRatingsWrapper {
    #[schema(description = "Index")]
    index: i32,
    #[schema(description = "TV Show Name")]
    show: StackString,
    #[schema(description = "Title")]
    title: Option<StackString>,
    #[schema(description = "IMDB ID")]
    link: StackString,
    #[schema(description = "Rating")]
    rating: Option<f64>,
    #[schema(description = "IsTv Flag")]
    istv: Option<bool>,
    #[schema(description = "Source")]
    source: Option<TvShowSourceWrapper>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct MovieQueueRowWrapper(MovieQueueRow);

derive_rweb_schema!(MovieQueueRowWrapper, _MovieQueueRowWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _MovieQueueRowWrapper {
    #[schema(description = "Queue Index")]
    idx: i32,
    #[schema(description = "Collection Index")]
    collection_idx: i32,
    #[schema(description = "Collection Path")]
    path: StackString,
    #[schema(description = "TV Show Name")]
    show: StackString,
    #[schema(description = "Last Modified")]
    last_modified: Option<DateTime<Utc>>,
}

#[derive(Default, Serialize, Deserialize, Into, From, Deref)]
pub struct MovieCollectionRowWrapper(MovieCollectionRow);

derive_rweb_schema!(MovieCollectionRowWrapper, _MovieCollectionRowWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _MovieCollectionRowWrapper {
    #[schema(description = "Collection Index")]
    idx: i32,
    #[schema(description = "Collection Path")]
    path: StackString,
    #[schema(description = "TV Show Name")]
    show: StackString,
}

#[derive(Serialize, Deserialize, Into, From)]
pub struct LastModifiedResponseWrapper(LastModifiedResponse);

derive_rweb_schema!(LastModifiedResponseWrapper, _LastModifiedResponseWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _LastModifiedResponseWrapper {
    #[schema(description = "Table Name")]
    table: StackString,
    #[schema(description = "Last Modified")]
    last_modified: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Into, From)]
pub struct PlexEventWrapper(PlexEvent);

derive_rweb_schema!(PlexEventWrapper, _PlexEventWrapper);

#[derive(Schema)]
pub struct _PlexEventWrapper {
    #[schema(description = "Event")]
    pub event: StackString,
    #[schema(description = "Account")]
    pub account: StackString,
    #[schema(description = "Server")]
    pub server: StackString,
    #[schema(description = "Player Title")]
    pub player_title: StackString,
    #[schema(description = "Player Address")]
    pub player_address: StackString,
    #[schema(description = "Title")]
    pub title: StackString,
    #[schema(description = "Parent Title")]
    pub parent_title: Option<StackString>,
    #[schema(description = "Grandparent Title")]
    pub grandparent_title: Option<StackString>,
    #[schema(description = "Added At Timestamp")]
    pub added_at: DateTime<Utc>,
    #[schema(description = "Updated At Timestamp")]
    pub updated_at: Option<DateTime<Utc>>,
    #[schema(description = "Last Modified")]
    pub last_modified: DateTime<Utc>,
    #[schema(description = "Metadata Type")]
    pub metadata_type: Option<StackString>,
    #[schema(description = "Section Type")]
    pub section_type: Option<StackString>,
    #[schema(description = "Section Title")]
    pub section_title: Option<StackString>,
    #[schema(description = "Metadata Key")]
    pub metadata_key: Option<StackString>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct PlexFilenameWrapper(PlexFilename);

derive_rweb_schema!(PlexFilenameWrapper, _PlexFilenameWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _PlexFilenameWrapper {
    #[schema(description = "Metadata Key")]
    metadata_key: StackString,
    #[schema(description = "Filename")]
    filename: StackString,
}

#[derive(Clone, Copy, Deserialize, Serialize, Into, From, Deref, FromStr, Display)]
pub struct TraktActionsWrapper(TraktActions);

derive_rweb_schema!(TraktActionsWrapper, _TraktActionsWrapper);

#[allow(dead_code)]
#[derive(Schema, Serialize)]
enum _TraktActionsWrapper {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "list")]
    List,
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "remove")]
    Remove,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Deref, Into, From)]
pub struct PlexEventTypeWrapper(PlexEventType);

derive_rweb_schema!(PlexEventTypeWrapper, _PlexEventTypeWrapper);

#[derive(Serialize, Deserialize, Schema)]
pub enum _PlexEventTypeWrapper {
    #[serde(rename = "library-on-deck")]
    LibraryOnDeck,
    #[serde(rename = "library-new")]
    LibraryNew,
    #[serde(rename = "media-pause")]
    MediaPause,
    #[serde(rename = "media-play")]
    MediaPlay,
    #[serde(rename = "media-rate")]
    MediaRate,
    #[serde(rename = "media-resume")]
    MediaResume,
    #[serde(rename = "media-scrobble")]
    MediaScrobble,
    #[serde(rename = "media-stop")]
    MediaStop,
    #[serde(rename = "admin-database-backup")]
    AdminDatabaseBackup,
    #[serde(rename = "admin-database-corrupted")]
    AdminDatabaseCorrupted,
    #[serde(rename = "device-new")]
    DeviceNew,
    #[serde(rename = "playback-started")]
    PlaybackStarted,
}

#[cfg(test)]
mod test {
    use rweb_helper::derive_rweb_test;

    use crate::{
        ImdbEpisodesWrapper, ImdbRatingsWrapper, LastModifiedResponseWrapper,
        MovieCollectionRowWrapper, MovieQueueRowWrapper, PlexEventTypeWrapper, PlexEventWrapper,
        PlexFilenameWrapper, TraktActionsWrapper, TvShowSourceWrapper, _ImdbEpisodesWrapper,
        _ImdbRatingsWrapper, _LastModifiedResponseWrapper, _MovieCollectionRowWrapper,
        _MovieQueueRowWrapper, _PlexEventTypeWrapper, _PlexEventWrapper, _PlexFilenameWrapper,
        _TraktActionsWrapper, _TvShowSourceWrapper,
    };

    #[test]
    fn test_type() {
        derive_rweb_test!(ImdbEpisodesWrapper, _ImdbEpisodesWrapper);
        derive_rweb_test!(TvShowSourceWrapper, _TvShowSourceWrapper);
        derive_rweb_test!(ImdbRatingsWrapper, _ImdbRatingsWrapper);
        derive_rweb_test!(MovieQueueRowWrapper, _MovieQueueRowWrapper);
        derive_rweb_test!(MovieCollectionRowWrapper, _MovieCollectionRowWrapper);
        derive_rweb_test!(LastModifiedResponseWrapper, _LastModifiedResponseWrapper);
        derive_rweb_test!(PlexEventWrapper, _PlexEventWrapper);
        derive_rweb_test!(PlexFilenameWrapper, _PlexFilenameWrapper);
        derive_rweb_test!(TraktActionsWrapper, _TraktActionsWrapper);
        derive_rweb_test!(PlexEventTypeWrapper, _PlexEventTypeWrapper);
    }
}
