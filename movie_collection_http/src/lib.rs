#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::manual_map)]
#![allow(clippy::unused_async)]
#![allow(clippy::implicit_hasher)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::ignored_unit_patterns)]

pub mod errors;
pub mod graphql;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_elements;
pub mod movie_queue_requests;
pub mod movie_queue_routes;

use derive_more::{Deref, Display, From, FromStr, Into};
use rweb::Schema;
use rweb_helper::{derive_rweb_schema, DateTimeType, DateType, DecimalWrapper, UuidWrapper};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use movie_collection_lib::{
    date_time_wrapper::DateTimeWrapper,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{LastModifiedResponse, MovieCollectionRow},
    movie_queue::{MovieQueueRow, OrderBy},
    music_collection::MusicCollection,
    plex_events::{PlexEvent, PlexEventType, PlexFilename, PlexMetadata, PlexSectionType},
    trakt_utils::TraktActions,
    tv_show_source::TvShowSource,
};

#[derive(Clone, Serialize, Deserialize, Into, From, Debug)]
pub struct ImdbEpisodesWrapper(ImdbEpisodes);

derive_rweb_schema!(ImdbEpisodesWrapper, _ImdbEpisodesWrapper);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "ImdbEpisodes")]
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
    airdate: Option<DateType>,
    #[schema(description = "Rating")]
    rating: Option<DecimalWrapper>,
    #[schema(description = "Episode Title")]
    eptitle: StackString,
    #[schema(description = "Episode URL")]
    epurl: StackString,
    #[schema(description = "ID")]
    id: UuidWrapper,
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
#[schema(component = "ImdbRatings")]
struct _ImdbRatingsWrapper {
    #[schema(description = "Index")]
    index: UuidWrapper,
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
#[schema(component = "MovieQueueRow")]
struct _MovieQueueRowWrapper {
    #[schema(description = "Queue Index")]
    idx: i32,
    #[schema(description = "Collection Index")]
    collection_idx: UuidWrapper,
    #[schema(description = "Collection Path")]
    path: StackString,
    #[schema(description = "TV Show Name")]
    show: StackString,
    #[schema(description = "Last Modified")]
    last_modified: Option<DateTimeType>,
}

#[derive(Default, Serialize, Deserialize, Into, From, Deref, Debug)]
pub struct MovieCollectionRowWrapper(MovieCollectionRow);

derive_rweb_schema!(MovieCollectionRowWrapper, _MovieCollectionRowWrapper);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "MovieCollectionRow")]
struct _MovieCollectionRowWrapper {
    #[schema(description = "Collection Index")]
    idx: UuidWrapper,
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
#[schema(component = "LastModifiedResponse")]
struct _LastModifiedResponseWrapper {
    #[schema(description = "Table Name")]
    table: StackString,
    #[schema(description = "Last Modified")]
    last_modified: DateTimeType,
}

#[derive(Debug, Serialize, Deserialize, Into, From)]
pub struct PlexEventWrapper(PlexEvent);

derive_rweb_schema!(PlexEventWrapper, _PlexEventWrapper);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "PlexEvent")]
struct _PlexEventWrapper {
    #[schema(description = "ID")]
    id: UuidWrapper,
    #[schema(description = "Event")]
    event: StackString,
    #[schema(description = "Account")]
    account: StackString,
    #[schema(description = "Server")]
    server: StackString,
    #[schema(description = "Player Title")]
    player_title: StackString,
    #[schema(description = "Player Address")]
    player_address: StackString,
    #[schema(description = "Title")]
    title: StackString,
    #[schema(description = "Parent Title")]
    parent_title: Option<StackString>,
    #[schema(description = "Grandparent Title")]
    grandparent_title: Option<StackString>,
    #[schema(description = "Added At Timestamp")]
    added_at: DateTimeType,
    #[schema(description = "Updated At Timestamp")]
    updated_at: Option<DateTimeType>,
    #[schema(description = "Last Modified")]
    last_modified: DateTimeType,
    #[schema(description = "Metadata Type")]
    metadata_type: Option<StackString>,
    #[schema(description = "Section Type")]
    section_type: Option<StackString>,
    #[schema(description = "Section Title")]
    section_title: Option<StackString>,
    #[schema(description = "Metadata Key")]
    metadata_key: Option<StackString>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct PlexFilenameWrapper(PlexFilename);

derive_rweb_schema!(PlexFilenameWrapper, _PlexFilenameWrapper);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "PlexFilename")]
struct _PlexFilenameWrapper {
    #[schema(description = "Metadata Key")]
    metadata_key: StackString,
    #[schema(description = "Filename")]
    filename: StackString,
    #[schema(description = "Collection Id")]
    collection_id: Option<UuidWrapper>,
    #[schema(description = "Music Collection Id")]
    music_collection_id: Option<UuidWrapper>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct PlexMetadataWrapper(PlexMetadata);

derive_rweb_schema!(PlexMetadataWrapper, _PlexMetadataWrapper);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "PlexMetadata")]
struct _PlexMetadataWrapper {
    #[schema(description = "Metadata Key")]
    metadata_key: StackString,
    #[schema(description = "Object Type")]
    object_type: StackString,
    #[schema(description = "Title")]
    title: StackString,
    #[schema(description = "Parent Key")]
    parent_key: Option<StackString>,
    #[schema(description = "Grandparent Key")]
    grandparent_key: Option<StackString>,
    #[schema(description = "show")]
    show: Option<StackString>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct MusicCollectionWrapper(MusicCollection);

derive_rweb_schema!(MusicCollectionWrapper, _MusicCollectionWrapper);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "MusicCollection")]
struct _MusicCollectionWrapper {
    #[schema(description = "Id")]
    id: UuidWrapper,
    #[schema(description = "Path")]
    path: StackString,
    #[schema(description = "Artist")]
    artist: Option<StackString>,
    #[schema(description = "Album")]
    album: Option<StackString>,
    #[schema(description = "Title")]
    title: Option<StackString>,
    #[schema(description = "Last Modified")]
    last_modified: DateTimeType,
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Deref, Into, From, PartialEq, Eq)]
pub struct OrderByWrapper(OrderBy);

derive_rweb_schema!(OrderByWrapper, _OrderByWrapper);

#[derive(Serialize, Deserialize, Schema)]
pub enum _OrderByWrapper {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Deref, Into, From)]
pub struct PlexSectionTypeWrapper(PlexSectionType);

derive_rweb_schema!(PlexSectionTypeWrapper, _PlexSectionTypeWrapper);

#[derive(Serialize, Deserialize, Schema)]
pub enum _PlexSectionTypeWrapper {
    #[serde(rename = "artist")]
    Music,
    #[serde(rename = "movie")]
    Movie,
    #[serde(rename = "show")]
    TvShow,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct PlexEventRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub event_type: Option<PlexEventTypeWrapper>,
    pub section_type: Option<PlexSectionTypeWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_rweb_schema!(PlexEventRequest, _PlexEventRequest);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "PlexEventRequest")]
struct _PlexEventRequest {
    #[schema(description = "Start Timestamp")]
    start_timestamp: Option<DateTimeType>,
    #[schema(description = "Event Type")]
    event_type: Option<PlexEventTypeWrapper>,
    #[schema(description = "Section Type")]
    section_type: Option<PlexSectionTypeWrapper>,
    #[schema(description = "Offset")]
    offset: Option<usize>,
    #[schema(description = "Limit")]
    limit: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PlexFilenameRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_rweb_schema!(PlexFilenameRequest, _PlexFilenameRequest);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "PlexEventFilenameRequest")]
struct _PlexFilenameRequest {
    #[schema(description = "Start Timestamp")]
    start_timestamp: Option<DateTimeType>,
    #[schema(description = "Offset")]
    offset: Option<usize>,
    #[schema(description = "Limit")]
    limit: Option<usize>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, Into, From)]
pub struct TraktWatchlistRequest {
    pub query: Option<StackString>,
    pub source: Option<TvShowSourceWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_rweb_schema!(TraktWatchlistRequest, _TraktWatchlistRequest);

#[allow(dead_code)]
#[derive(Schema)]
#[schema(component = "TraktWatchlistRequest")]
pub struct _TraktWatchlistRequest {
    #[schema(description = "Search Query")]
    pub query: Option<StackString>,
    #[schema(description = "Tv Show Source")]
    pub source: Option<TvShowSourceWrapper>,
    #[schema(description = "Offset")]
    pub offset: Option<StackString>,
    #[schema(description = "Limit")]
    pub limit: Option<StackString>,
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
