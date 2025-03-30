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
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use time::{Date, OffsetDateTime};
use utoipa::ToSchema;
use utoipa_helper::derive_utoipa_schema;
use uuid::Uuid;

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

derive_utoipa_schema!(ImdbEpisodesWrapper, _ImdbEpisodesWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = ImdbEpisodes)]
// ImdbEpisodes")]
struct _ImdbEpisodesWrapper {
    // TV Show Name")]
    show: StackString,
    // Title")]
    title: StackString,
    // Season")]
    season: i32,
    // Episode")]
    episode: i32,
    // Airdate")]
    airdate: Option<Date>,
    // Rating")]
    rating: Option<Decimal>,
    // Episode Title")]
    eptitle: StackString,
    // Episode URL")]
    epurl: StackString,
    // ID")]
    id: Uuid,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, Copy, PartialEq, Into, From)]
pub struct TvShowSourceWrapper(TvShowSource);

derive_utoipa_schema!(TvShowSourceWrapper, _TvShowSourceWrapper);

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
#[schema(as = TvShowSource)]
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

derive_utoipa_schema!(ImdbRatingsWrapper, _ImdbRatingsWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = ImdbRatings)]
// ImdbRatings")]
struct _ImdbRatingsWrapper {
    // Index")]
    index: Uuid,
    // TV Show Name")]
    show: StackString,
    // Title")]
    title: Option<StackString>,
    // IMDB ID")]
    link: StackString,
    // Rating")]
    rating: Option<f64>,
    // IsTv Flag")]
    istv: Option<bool>,
    // Source")]
    source: Option<TvShowSourceWrapper>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct MovieQueueRowWrapper(MovieQueueRow);

derive_utoipa_schema!(MovieQueueRowWrapper, _MovieQueueRowWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = MovieQueueRow)]
// MovieQueueRow")]
struct _MovieQueueRowWrapper {
    // Queue Index")]
    idx: i32,
    // Collection Index")]
    collection_idx: Uuid,
    // Collection Path")]
    path: StackString,
    // TV Show Name")]
    show: StackString,
    // Last Modified")]
    last_modified: Option<OffsetDateTime>,
}

#[derive(Default, Serialize, Deserialize, Into, From, Deref, Debug)]
pub struct MovieCollectionRowWrapper(MovieCollectionRow);

derive_utoipa_schema!(MovieCollectionRowWrapper, _MovieCollectionRowWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = MovieCollectionRow)]
// MovieCollectionRow")]
struct _MovieCollectionRowWrapper {
    // Collection Index")]
    idx: Uuid,
    // Collection Path")]
    path: StackString,
    // TV Show Name")]
    show: StackString,
}

#[derive(Serialize, Deserialize, Into, From)]
pub struct LastModifiedResponseWrapper(LastModifiedResponse);

derive_utoipa_schema!(LastModifiedResponseWrapper, _LastModifiedResponseWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = LastModifiedResponse)]
// LastModifiedResponse")]
struct _LastModifiedResponseWrapper {
    // Table Name")]
    table: StackString,
    // Last Modified")]
    last_modified: OffsetDateTime,
}

#[derive(Debug, Serialize, Deserialize, Into, From)]
pub struct PlexEventWrapper(PlexEvent);

derive_utoipa_schema!(PlexEventWrapper, _PlexEventWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = PlexEvent)]
// PlexEvent")]
struct _PlexEventWrapper {
    // ID")]
    id: Uuid,
    // Event")]
    event: StackString,
    // Account")]
    account: StackString,
    // Server")]
    server: StackString,
    // Player Title")]
    player_title: StackString,
    // Player Address")]
    player_address: StackString,
    // Title")]
    title: StackString,
    // Parent Title")]
    parent_title: Option<StackString>,
    // Grandparent Title")]
    grandparent_title: Option<StackString>,
    // Added At Timestamp")]
    added_at: OffsetDateTime,
    // Updated At Timestamp")]
    updated_at: Option<OffsetDateTime>,
    // Last Modified")]
    last_modified: OffsetDateTime,
    // Metadata Type")]
    metadata_type: Option<StackString>,
    // Section Type")]
    section_type: Option<StackString>,
    // Section Title")]
    section_title: Option<StackString>,
    // Metadata Key")]
    metadata_key: Option<StackString>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct PlexFilenameWrapper(PlexFilename);

derive_utoipa_schema!(PlexFilenameWrapper, _PlexFilenameWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = PlexFilename)]
// PlexFilename")]
struct _PlexFilenameWrapper {
    // Metadata Key")]
    metadata_key: StackString,
    // Filename")]
    filename: StackString,
    // Collection Id")]
    collection_id: Option<Uuid>,
    // Music Collection Id")]
    music_collection_id: Option<Uuid>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct PlexMetadataWrapper(PlexMetadata);

derive_utoipa_schema!(PlexMetadataWrapper, _PlexMetadataWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = PlexMetadata)]
// PlexMetadata")]
struct _PlexMetadataWrapper {
    // Metadata Key")]
    metadata_key: StackString,
    // Object Type")]
    object_type: StackString,
    // Title")]
    title: StackString,
    // Parent Key")]
    parent_key: Option<StackString>,
    // Grandparent Key")]
    grandparent_key: Option<StackString>,
    // show")]
    show: Option<StackString>,
}

#[derive(Default, Debug, Serialize, Deserialize, Into, From, Deref)]
pub struct MusicCollectionWrapper(MusicCollection);

derive_utoipa_schema!(MusicCollectionWrapper, _MusicCollectionWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = MusicCollection)]
// MusicCollection")]
struct _MusicCollectionWrapper {
    // Id")]
    id: Uuid,
    // Path")]
    path: StackString,
    // Artist")]
    artist: Option<StackString>,
    // Album")]
    album: Option<StackString>,
    // Title")]
    title: Option<StackString>,
    // Last Modified")]
    last_modified: OffsetDateTime,
}

#[derive(Clone, Copy, Deserialize, Serialize, Into, From, Deref, FromStr, Display)]
pub struct TraktActionsWrapper(TraktActions);

derive_utoipa_schema!(TraktActionsWrapper, _TraktActionsWrapper);

#[allow(dead_code)]
#[derive(ToSchema, Serialize)]
#[schema(as = TraktActions)]
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

derive_utoipa_schema!(OrderByWrapper, _OrderByWrapper);

#[derive(Serialize, Deserialize, ToSchema)]
#[schema(as = OrderBy)]
pub enum _OrderByWrapper {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Deref, Into, From)]
pub struct PlexSectionTypeWrapper(PlexSectionType);

derive_utoipa_schema!(PlexSectionTypeWrapper, _PlexSectionTypeWrapper);

#[derive(Serialize, Deserialize, ToSchema)]
#[schema(as = PlexSectionType)]
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

derive_utoipa_schema!(PlexEventTypeWrapper, _PlexEventTypeWrapper);

#[derive(Serialize, Deserialize, ToSchema)]
#[schema(as = PlexEventType)]
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

derive_utoipa_schema!(PlexEventRequest, _PlexEventRequest);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = PlexEventRequest)]
// PlexEventRequest")]
struct _PlexEventRequest {
    // Start Timestamp")]
    start_timestamp: Option<OffsetDateTime>,
    // Event Type")]
    event_type: Option<PlexEventTypeWrapper>,
    // Section Type")]
    section_type: Option<PlexSectionTypeWrapper>,
    // Offset")]
    offset: Option<usize>,
    // Limit")]
    limit: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PlexFilenameRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_utoipa_schema!(PlexFilenameRequest, _PlexFilenameRequest);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = PlexFilenameRequest)]
// PlexEventFilenameRequest")]
struct _PlexFilenameRequest {
    // Start Timestamp")]
    start_timestamp: Option<OffsetDateTime>,
    // Offset")]
    offset: Option<usize>,
    // Limit")]
    limit: Option<usize>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, Into, From)]
pub struct TraktWatchlistRequest {
    pub query: Option<StackString>,
    pub source: Option<TvShowSourceWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_utoipa_schema!(TraktWatchlistRequest, _TraktWatchlistRequest);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = TraktWatchlistRequest)]
// TraktWatchlistRequest")]
pub struct _TraktWatchlistRequest {
    // Search Query")]
    pub query: Option<StackString>,
    // Tv Show Source")]
    pub source: Option<TvShowSourceWrapper>,
    // Offset")]
    pub offset: Option<StackString>,
    // Limit")]
    pub limit: Option<StackString>,
}

#[cfg(test)]
mod test {
    use utoipa_helper::derive_utoipa_test;

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
        derive_utoipa_test!(ImdbEpisodesWrapper, _ImdbEpisodesWrapper);
        derive_utoipa_test!(TvShowSourceWrapper, _TvShowSourceWrapper);
        derive_utoipa_test!(ImdbRatingsWrapper, _ImdbRatingsWrapper);
        derive_utoipa_test!(MovieQueueRowWrapper, _MovieQueueRowWrapper);
        derive_utoipa_test!(MovieCollectionRowWrapper, _MovieCollectionRowWrapper);
        derive_utoipa_test!(LastModifiedResponseWrapper, _LastModifiedResponseWrapper);
        derive_utoipa_test!(PlexEventWrapper, _PlexEventWrapper);
        derive_utoipa_test!(PlexFilenameWrapper, _PlexFilenameWrapper);
        derive_utoipa_test!(TraktActionsWrapper, _TraktActionsWrapper);
        derive_utoipa_test!(PlexEventTypeWrapper, _PlexEventTypeWrapper);
    }
}
