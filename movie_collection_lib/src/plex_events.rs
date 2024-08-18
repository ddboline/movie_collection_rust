use anyhow::{format_err, Error};
use futures::{Stream, TryStreamExt};
use log::info;
use postgres_query::{
    client::GenericClient, query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query,
};
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    convert::{TryFrom, TryInto},
    fmt,
    net::Ipv4Addr,
    path::Path,
    str::FromStr,
};
use time::{macros::datetime, OffsetDateTime};
use uuid::Uuid;

use crate::{
    config::Config,
    date_time_wrapper::DateTimeWrapper,
    pgpool::{PgPool, PgTransaction},
};

#[derive(FromSqlRow, Debug, Serialize, Deserialize)]
pub struct PlexEvent {
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

impl TryFrom<WebhookPayload> for PlexEvent {
    type Error = Error;
    fn try_from(item: WebhookPayload) -> Result<Self, Self::Error> {
        fn dt_from_tm(x: u64) -> OffsetDateTime {
            OffsetDateTime::from_unix_timestamp(x as i64)
                .unwrap_or(datetime!(1970-01-01 00:00:00 +00:00))
        }
        let event = item.event.to_str().into();
        let player_address = StackString::from_display(item.player.public_address);
        let payload = Self {
            event,
            account: item.account.title,
            server: item.server.title,
            player_title: item.player.title,
            player_address,
            title: item.metadata.title,
            parent_title: item.metadata.parent_title,
            grandparent_title: item.metadata.grandparent_title,
            added_at: dt_from_tm(item.metadata.added_at).into(),
            updated_at: item.metadata.updated_at.map(dt_from_tm).map(Into::into),
            last_modified: DateTimeWrapper::now(),
            metadata_type: item.metadata.metadata_type,
            section_type: item.metadata.library_section_type,
            section_title: item.metadata.library_section_title,
            metadata_key: item.metadata.key,
        };
        Ok(payload)
    }
}

#[derive(FromSqlRow, PartialEq, Eq, Clone)]
pub struct EventOutput {
    pub id: Uuid,
    pub event: StackString,
    pub metadata_type: Option<StackString>,
    pub section_title: Option<StackString>,
    pub title: StackString,
    pub parent_title: Option<StackString>,
    pub grandparent_title: Option<StackString>,
    pub filename: Option<StackString>,
    pub last_modified: OffsetDateTime,
}

impl PlexEvent {
    /// # Errors
    /// Return error if deserialization fails
    pub fn get_from_payload(buf: &[u8]) -> Result<Self, Error> {
        info!(
            "buf {}",
            if let Ok(s) = std::str::from_utf8(buf) {
                s
            } else {
                ""
            }
        );
        let object: WebhookPayload = serde_json::from_slice(buf)?;
        info!("{:#?}", object);
        object.try_into()
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_events(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
        event_type: Option<PlexEventType>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let mut constraints = Vec::new();
        let mut bindings = Vec::new();
        if let Some(start_timestamp) = &start_timestamp {
            constraints.push("last_modified > $start_timestamp");
            bindings.push(("start_timestamp", start_timestamp as Parameter));
        }
        let event_type: Option<StackString> = event_type.map(|s| s.to_str().into());
        if let Some(event_type) = &event_type {
            constraints.push("event = $event");
            bindings.push(("event", event_type as Parameter));
        }
        let query = format_sstr!(
            "
                SELECT * FROM plex_event
                {where_str}
                ORDER BY last_modified DESC
                {limit}
                {offset}
            ",
            where_str = if constraints.is_empty() {
                StackString::new()
            } else {
                format_sstr!("WHERE {}", constraints.join(" AND "))
            },
            limit = if let Some(limit) = limit {
                format_sstr!("LIMIT {limit}")
            } else {
                StackString::new()
            },
            offset = if let Some(offset) = offset {
                format_sstr!("OFFSET {offset}")
            } else {
                StackString::new()
            }
        );
        let query: Query = query_dyn!(&query, ..bindings)?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn write_event(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
            INSERT INTO plex_event (
                event, account, server, player_title, player_address, title, parent_title,
                grandparent_title, added_at, updated_at, last_modified, metadata_type,
                section_type, section_title, metadata_key
            )
            VALUES (
                $event, $account, $server, $player_title, $player_address, $title, $parent_title,
                $grandparent_title, $added_at, $updated_at, $last_modified,
                $metadata_type, $section_type, $section_title, $metadata_key
            )
            ",
            event = self.event,
            account = self.account,
            server = self.server,
            player_title = self.player_title,
            player_address = self.player_address,
            title = self.title,
            parent_title = self.parent_title,
            grandparent_title = self.grandparent_title,
            added_at = self.added_at,
            updated_at = self.updated_at,
            last_modified = self.last_modified,
            metadata_type = self.metadata_type,
            section_type = self.section_type,
            section_title = self.section_title,
            metadata_key = self.metadata_key,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_event_by_id(pool: &PgPool, id: Uuid) -> Result<Option<EventOutput>, Error> {
        let query = query!(
            r#"
                SELECT a.id, a.event, a.metadata_type, a.section_title, a.title, a.parent_title,
                       a.grandparent_title, b.filename, a.last_modified
                FROM plex_event a
                LEFT JOIN plex_filename b ON a.metadata_key = b.metadata_key
                WHERE a.id = $id
            "#,
            id = id,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_plex_events(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
        event_type: Option<PlexEventType>,
        section_type: Option<PlexSectionType>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Vec<EventOutput>, Error> {
        let mut constraints = Vec::new();
        let mut bindings = Vec::new();
        if let Some(start_timestamp) = &start_timestamp {
            constraints.push("a.last_modified > $start_timestamp");
            bindings.push(("start_timestamp", start_timestamp as Parameter));
        }
        let event_type: Option<StackString> = event_type.map(|s| s.to_str().into());
        if let Some(event_type) = &event_type {
            constraints.push("a.event = $event");
            bindings.push(("event", event_type as Parameter));
        }
        let section_type: Option<StackString> = section_type.map(|s| s.to_str().into());
        if let Some(section_type) = &section_type {
            constraints.push("a.section_type = $section_type");
            bindings.push(("section_type", section_type as Parameter));
        }
        let query = format_sstr!(
            "
                SELECT a.id, a.event, a.metadata_type, a.section_title, a.title, a.parent_title,
                       a.grandparent_title, b.filename, a.last_modified
                FROM plex_event a
                LEFT JOIN plex_filename b ON a.metadata_key = b.metadata_key
                {where_str}
                ORDER BY a.last_modified DESC
                {limit}
                {offset}
            ",
            where_str = if constraints.is_empty() {
                StackString::new()
            } else {
                format_sstr!("WHERE {}", constraints.join(" AND "))
            },
            limit = if let Some(limit) = limit {
                format_sstr!("LIMIT {limit}")
            } else {
                StackString::new()
            },
            offset = if let Some(offset) = offset {
                format_sstr!("OFFSET {offset}")
            } else {
                StackString::new()
            }
        );
        let query: Query = query_dyn!(&query, ..bindings)?;
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_filename(&self, config: &Config) -> Result<PlexFilename, Error> {
        let plex_host = config
            .plex_host
            .as_ref()
            .ok_or_else(|| format_err!("No Host"))?;
        let plex_token = config
            .plex_token
            .as_ref()
            .ok_or_else(|| format_err!("No Token"))?;
        let metadata_key = self
            .metadata_key
            .as_ref()
            .ok_or_else(|| format_err!("No metadata_key"))?;
        let url = format_sstr!("http://{plex_host}:32400{metadata_key}?X-Plex-Token={plex_token}");
        let data = reqwest::get(url.as_str())
            .await?
            .error_for_status()?
            .text()
            .await?;
        let filename = Self::extract_filename_from_xml(&data)?;
        Ok(PlexFilename {
            metadata_key: metadata_key.clone(),
            filename,
            collection_id: None,
        })
    }

    fn extract_filename_from_xml(xml: &str) -> Result<StackString, Error> {
        let doc = Document::parse(xml)?;
        doc.descendants()
            .find_map(|n| n.attribute("file").map(Into::into))
            .ok_or_else(|| format_err!("No file found"))
    }
}

#[derive(Deserialize, Debug)]
pub struct Account {
    pub id: isize,
    pub thumb: StackString,
    pub title: StackString,
}

#[derive(Deserialize, Debug)]
pub struct Server {
    pub title: StackString,
    pub uuid: StackString,
}

#[derive(Deserialize, Debug)]
pub struct Player {
    pub local: bool,
    #[serde(alias = "publicAddress")]
    pub public_address: Ipv4Addr,
    pub title: StackString,
    pub uuid: StackString,
}

#[derive(Deserialize, Debug)]
pub struct Metadata {
    #[serde(alias = "type")]
    pub metadata_type: Option<StackString>,
    pub title: StackString,
    #[serde(alias = "parentTitle")]
    pub parent_title: Option<StackString>,
    #[serde(alias = "grandparentTitle")]
    pub grandparent_title: Option<StackString>,
    pub summary: Option<StackString>,
    pub rating: Option<f64>,
    #[serde(alias = "ratingCount")]
    pub rating_count: Option<u64>,
    pub key: Option<StackString>,
    #[serde(alias = "parentKey")]
    pub parent_key: Option<StackString>,
    #[serde(alias = "grandparentKey")]
    pub grandparent_key: Option<StackString>,
    #[serde(alias = "addedAt")]
    pub added_at: u64,
    #[serde(alias = "updatedAt")]
    pub updated_at: Option<u64>,
    #[serde(alias = "librarySectionType")]
    pub library_section_type: Option<StackString>,
    #[serde(alias = "librarySectionTitle")]
    pub library_section_title: Option<StackString>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlexSectionType {
    #[serde(alias = "artist")]
    Music,
    #[serde(alias = "movie")]
    Movie,
    #[serde(alias = "show")]
    TvShow,
}

impl PlexSectionType {
    #[must_use]
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Music => "artist",
            Self::Movie => "movie",
            Self::TvShow => "show",
        }
    }

    #[must_use]
    pub fn to_display(self) -> &'static str {
        match self {
            Self::Music => "Music",
            Self::Movie => "Movie",
            Self::TvShow => "TvShow",
        }
    }
}

impl fmt::Display for PlexSectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

impl FromStr for PlexSectionType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "artist" => Ok(Self::Music),
            "movie" => Ok(Self::Movie),
            "show" => Ok(Self::TvShow),
            _ => Err(format_err!("Invalid PlexSectionType")),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum PlexEventType {
    #[serde(alias = "library.on.deck")]
    LibraryOnDeck,
    #[serde(alias = "library.new")]
    LibraryNew,
    #[serde(alias = "media.pause")]
    MediaPause,
    #[serde(alias = "media.play")]
    MediaPlay,
    #[serde(alias = "media.rate")]
    MediaRate,
    #[serde(alias = "media.resume")]
    MediaResume,
    #[serde(alias = "media.scrobble")]
    MediaScrobble,
    #[serde(alias = "media.stop")]
    MediaStop,
    #[serde(alias = "admin.database.backup")]
    AdminDatabaseBackup,
    #[serde(alias = "admin.database.corrupted")]
    AdminDatabaseCorrupted,
    #[serde(alias = "device.new")]
    DeviceNew,
    #[serde(alias = "playback.started")]
    PlaybackStarted,
}

impl PlexEventType {
    #[must_use]
    pub fn to_str(self) -> &'static str {
        match self {
            Self::LibraryOnDeck => "library.on.deck",
            Self::LibraryNew => "library.new",
            Self::MediaPause => "media.pause",
            Self::MediaPlay => "media.play",
            Self::MediaRate => "media.rate",
            Self::MediaResume => "media.resume",
            Self::MediaScrobble => "media.scrobble",
            Self::MediaStop => "media.stop",
            Self::AdminDatabaseBackup => "admin.database.backup",
            Self::AdminDatabaseCorrupted => "admin.database.corrupted",
            Self::DeviceNew => "device.new",
            Self::PlaybackStarted => "playback.started",
        }
    }
}

impl FromStr for PlexEventType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "library.on.deck" => Ok(Self::LibraryOnDeck),
            "library.new" => Ok(Self::LibraryNew),
            "media.pause" => Ok(Self::MediaPause),
            "media.play" => Ok(Self::MediaPlay),
            "media.rate" => Ok(Self::MediaRate),
            "media.resume" => Ok(Self::MediaResume),
            "media.scrobble" => Ok(Self::MediaScrobble),
            "media.stop" => Ok(Self::MediaStop),
            "admin.database.backup" => Ok(Self::AdminDatabaseBackup),
            "admin.database.corrupted" => Ok(Self::AdminDatabaseCorrupted),
            "device.new" => Ok(Self::DeviceNew),
            "playback.started" => Ok(Self::PlaybackStarted),
            _ => Err(format_err!("Invalid PlexEventType")),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct WebhookPayload {
    pub event: PlexEventType,
    pub user: bool,
    pub owner: bool,
    #[serde(alias = "Account")]
    pub account: Account,
    #[serde(alias = "Server")]
    pub server: Server,
    #[serde(alias = "Player")]
    pub player: Player,
    #[serde(alias = "Metadata")]
    pub metadata: Metadata,
}

#[derive(FromSqlRow, Default, Debug, Serialize, Deserialize)]
pub struct PlexFilename {
    pub metadata_key: StackString,
    pub filename: StackString,
    pub collection_id: Option<Uuid>,
}

impl PlexFilename {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_filenames(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let mut constraints = Vec::new();
        let mut bindings = Vec::new();
        if let Some(start_timestamp) = &start_timestamp {
            constraints.push("last_modified > $start_timestamp");
            bindings.push(("start_timestamp", start_timestamp as Parameter));
        }
        let query = format_sstr!(
            "
                SELECT * FROM plex_filename
                {where_str}
                ORDER BY last_modified DESC
                {limit}
                {offset}
            ",
            where_str = if constraints.is_empty() {
                StackString::new()
            } else {
                format_sstr!("WHERE {}", constraints.join(" AND "))
            },
            limit = if let Some(limit) = limit {
                format_sstr!("LIMIT {limit}")
            } else {
                StackString::new()
            },
            offset = if let Some(offset) = offset {
                format_sstr!("OFFSET {offset}")
            } else {
                StackString::new()
            }
        );
        let query: Query = query_dyn!(&query, ..bindings)?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    async fn _get_by_key<C>(conn: &C, key: &str) -> Result<Option<Self>, Error>
    where
        C: GenericClient + Sync,
    {
        let query = query!(
            "SELECT * FROM plex_filename WHERE metadata_key = $key",
            key = key,
        );
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_key(pool: &PgPool, key: &str) -> Result<Option<Self>, Error> {
        let conn = pool.get().await?;
        Self::_get_by_key(&conn, key).await
    }

    async fn _insert<C>(&self, conn: &C) -> Result<(), Error>
    where
        C: GenericClient + Sync,
    {
        let query = query!(
            "INSERT INTO plex_filename (metadata_key, filename)
            VALUES ($metadata_key, $filename)",
            metadata_key = self.metadata_key,
            filename = self.filename,
        );
        query.execute(&conn).await?;
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert(&self, pool: &PgPool) -> Result<(), Error> {
        let mut conn = pool.get().await?;
        let tran = conn.transaction().await?;
        let conn: &PgTransaction = &tran;

        if Self::_get_by_key(&conn, &self.metadata_key)
            .await?
            .is_none()
        {
            self._insert(&conn).await?;
        }
        tran.commit().await?;
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn delete(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "DELETE FROM plex_filename WHERE metadata_key=$key",
            key = self.metadata_key
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }
}

#[derive(Default, Debug, FromSqlRow, Serialize, Deserialize)]
pub struct PlexMetadata {
    pub metadata_key: StackString,
    pub object_type: StackString,
    pub title: StackString,
    pub parent_key: Option<StackString>,
    pub grandparent_key: Option<StackString>,
    pub show: Option<StackString>,
}

impl PlexMetadata {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_entries(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let mut constraints = Vec::new();
        let mut bindings = Vec::new();
        if let Some(start_timestamp) = &start_timestamp {
            constraints.push("last_modified > $start_timestamp");
            bindings.push(("start_timestamp", start_timestamp as Parameter));
        }
        let query = format_sstr!(
            "
                SELECT * FROM plex_metadata
                {where_str}
                ORDER BY last_modified DESC
                {limit}
                {offset}
            ",
            where_str = if constraints.is_empty() {
                StackString::new()
            } else {
                format_sstr!("WHERE {}", constraints.join(" AND "))
            },
            limit = if let Some(limit) = limit {
                format_sstr!("LIMIT {limit}")
            } else {
                StackString::new()
            },
            offset = if let Some(offset) = offset {
                format_sstr!("OFFSET {offset}")
            } else {
                StackString::new()
            }
        );
        let query: Query = query_dyn!(&query, ..bindings)?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    async fn get_parents(
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!(
            r#"
                SELECT *
                FROM plex_metadata
                WHERE metadata_key IN (
                    SELECT distinct parent_key
                    FROM plex_metadata
                    WHERE object_type = 'video'
                      AND parent_key IS NOT NULL
                    UNION
                    SELECT distinct grandparent_key
                    FROM plex_metadata
                    WHERE object_type = 'video'
                      AND grandparent_key IS NOT NULL
                )
            "#
        );
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_key(pool: &PgPool, key: &str) -> Result<Option<Self>, Error> {
        let conn = pool.get().await?;
        Self::_get_by_key(&conn, key).await
    }

    async fn _get_by_key<C>(conn: &C, key: &str) -> Result<Option<Self>, Error>
    where
        C: GenericClient + Sync,
    {
        let query = query!(
            "SELECT * FROM plex_metadata WHERE metadata_key = $key",
            key = key,
        );
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    async fn _insert<C>(&self, conn: &C) -> Result<u64, Error>
    where
        C: GenericClient + Sync,
    {
        let query = query!(
            r#"
                INSERT INTO plex_metadata (
                    metadata_key, object_type, title, parent_key, grandparent_key
                ) VALUES (
                    $metadata_key, $object_type, $title, $parent_key, $grandparent_key
                )
            "#,
            metadata_key = self.metadata_key,
            object_type = self.object_type,
            title = self.title,
            parent_key = self.parent_key,
            grandparent_key = self.grandparent_key,
        );
        query.execute(&conn).await.map_err(Into::into)
    }

    async fn _update<C>(&self, conn: &C) -> Result<u64, Error>
    where
        C: GenericClient + Sync,
    {
        let query = query!(
            r#"
                UPDATE plex_metadata
                SET object_type=$object_type,
                    title=$title,
                    parent_key=$parent_key,
                    grandparent_key=$grandparent_key,
                    show=$show,
                    last_modified=now()
                WHERE metadata_key=$metadata_key
            "#,
            metadata_key = self.metadata_key,
            object_type = self.object_type,
            title = self.title,
            parent_key = self.parent_key,
            grandparent_key = self.grandparent_key,
            show = self.show,
        );
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert(&self, pool: &PgPool) -> Result<u64, Error> {
        let mut conn = pool.get().await?;
        let tran = conn.transaction().await?;
        let conn: &PgTransaction = &tran;

        let bytes = match Self::_get_by_key(&conn, &self.metadata_key).await? {
            Some(_) => self._update(&conn).await?,
            None => self._insert(&conn).await?,
        };
        tran.commit().await?;
        Ok(bytes)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_metadata_by_key(config: &Config, key: &str) -> Result<Self, Error> {
        let plex_host = config
            .plex_host
            .as_ref()
            .ok_or_else(|| format_err!("No Host"))?;
        let plex_token = config
            .plex_token
            .as_ref()
            .ok_or_else(|| format_err!("No Token"))?;
        let url = format_sstr!("http://{plex_host}:32400{key}?X-Plex-Token={plex_token}");
        let data = reqwest::get(url.as_str())
            .await?
            .error_for_status()?
            .text()
            .await?;
        let metadata = Self::extract_metadata_from_xml(&data)?;
        Ok(metadata)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_children(
        &self,
        config: &Config,
    ) -> Result<Vec<(Self, Option<PlexFilename>)>, Error> {
        let plex_host = config
            .plex_host
            .as_ref()
            .ok_or_else(|| format_err!("No Host"))?;
        let plex_token = config
            .plex_token
            .as_ref()
            .ok_or_else(|| format_err!("No Token"))?;
        let key = &self.metadata_key;
        let url = format_sstr!("http://{plex_host}:32400{key}/children?X-Plex-Token={plex_token}");
        let data = reqwest::get(url.as_str())
            .await?
            .error_for_status()?
            .text()
            .await?;
        Self::extract_children_from_xml(&data)
    }

    async fn fill_plex_metadata_show(pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                UPDATE plex_metadata
                SET show = (
                    SELECT mc.show
                    FROM plex_filename pf
                    JOIN movie_collection mc ON mc.idx = pf.collection_id
                    JOIN plex_metadata pm ON pm.metadata_key = pf.metadata_key
                    JOIN imdb_ratings ir ON ir.show = mc.show
                    WHERE pf.metadata_key = plex_metadata.metadata_key
                ),last_modified=now()
                WHERE show IS NULL
            "#
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    async fn fill_plex_parent_metadata_show(pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                UPDATE plex_metadata
                SET show = (
                    SELECT distinct mc.show
                    FROM plex_metadata pmp
                    JOIN plex_metadata pm ON pm.parent_key = pmp.metadata_key
                    JOIN plex_filename pf ON pf.metadata_key = pm.metadata_key
                    JOIN movie_collection mc ON mc.idx = pf.collection_id
                    WHERE pmp.metadata_key = plex_metadata.metadata_key
                    LIMIT 1
                ),last_modified=now()
                WHERE show IS NULL
                AND object_type = 'directory'
            "#
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    async fn fill_plex_grandparent_metadata_show(pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                UPDATE plex_metadata
                SET show = (
                    SELECT distinct mc.show
                    FROM plex_metadata pmgp
                    JOIN plex_metadata pm ON pm.grandparent_key = pmgp.metadata_key
                    JOIN plex_filename pf ON pf.metadata_key = pm.metadata_key
                    JOIN movie_collection mc ON mc.idx = pf.collection_id
                    WHERE pmgp.metadata_key = plex_metadata.metadata_key
                    LIMIT 1
                ),last_modified=now()
                WHERE show IS NULL
                AND object_type = 'directory'
            "#
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn fill_plex_metadata(pool: &PgPool, config: &Config) -> Result<(), Error> {
        let bytes_written = Self::fill_plex_metadata_show(pool).await?;
        println!("update metadata {bytes_written}");
        let bytes_written = Self::fill_plex_parent_metadata_show(pool).await?;
        println!("update parent {bytes_written}");
        let bytes_written = Self::fill_plex_grandparent_metadata_show(pool).await?;
        println!("update grandparent {bytes_written}");
        let filenames: Vec<_> = PlexFilename::get_filenames(pool, None, None, None)
            .await?
            .try_collect()
            .await?;
        for plex_filename in filenames {
            if let Some(metadata) = Self::get_by_key(pool, &plex_filename.metadata_key).await? {
                if let Some(parent_key) = &metadata.parent_key {
                    if Self::get_by_key(pool, parent_key).await?.is_none() {
                        let mut parent_metadata =
                            Self::get_metadata_by_key(config, parent_key).await?;
                        parent_metadata.show = metadata.show.clone();
                        parent_metadata.insert(pool).await?;
                        println!("insert parent {parent_metadata:?}");
                    }
                }
                if let Some(grandparent_key) = &metadata.grandparent_key {
                    if Self::get_by_key(pool, grandparent_key).await?.is_none() {
                        let mut grandparent_metadata =
                            Self::get_metadata_by_key(config, grandparent_key).await?;
                        grandparent_metadata.show = metadata.show.clone();
                        grandparent_metadata.insert(pool).await?;
                        println!("insert grandparent {grandparent_metadata:?}");
                    }
                }
                continue;
            }
            let true_filename = plex_filename.filename.replace("/shares/", "/media/");
            if !config.movie_dirs.is_empty() && !Path::new(&true_filename).exists() {
                println!("file doesnt exist {true_filename}");
                plex_filename.delete(pool).await?;
                continue;
            }
            match Self::get_metadata_by_key(config, &plex_filename.metadata_key).await {
                Ok(metadata) => {
                    metadata.insert(pool).await?;
                    println!("insert {metadata:?}");
                }
                Err(e) => {
                    println!(
                        "encounterd error {e} key {} filename {}",
                        plex_filename.metadata_key, plex_filename.filename
                    );
                    plex_filename.delete(pool).await?;
                }
            }
        }
        let parents: Vec<_> = Self::get_parents(pool).await?.try_collect().await?;
        for parent in parents {
            for (metadata, filename) in parent.get_children(config).await? {
                if Self::get_by_key(pool, &metadata.metadata_key)
                    .await?
                    .is_none()
                {
                    metadata.insert(pool).await?;
                    println!("insert {metadata:?}");
                }
                if let Some(filename) = filename {
                    if PlexFilename::get_by_key(pool, &filename.metadata_key)
                        .await?
                        .is_none()
                    {
                        filename.insert(pool).await?;
                        println!("insert {filename:?}");
                    }
                }
            }
        }
        Ok(())
    }

    fn extract_metadata_from_xml(xml: &str) -> Result<Self, Error> {
        let doc = Document::parse(xml)?;
        for d in doc.descendants() {
            let object_type = match d.tag_name().name() {
                "Video" => "video",
                "Directory" => "directory",
                "Track" => "track",
                _ => continue,
            }
            .into();
            let metadata_key = d
                .attribute("key")
                .ok_or_else(|| format_err!("no key"))?
                .trim_end_matches("/children")
                .into();
            let title = d
                .attribute("title")
                .ok_or_else(|| format_err!("no title"))?
                .into();
            let parent_key = d.attribute("parentKey").map(Into::into);
            let grandparent_key = d.attribute("grandparentKey").map(Into::into);
            return Ok(PlexMetadata {
                metadata_key,
                object_type,
                title,
                parent_key,
                grandparent_key,
                ..PlexMetadata::default()
            });
        }
        Err(format_err!("No metadata found"))
    }

    fn extract_children_from_xml(xml: &str) -> Result<Vec<(Self, Option<PlexFilename>)>, Error> {
        let doc = Document::parse(xml)?;
        doc.descendants()
            .map(|d| {
                let object_type = match d.tag_name().name() {
                    "Video" => "video",
                    "Directory" => "directory",
                    "Track" => "track",
                    _ => return Ok(None),
                }
                .into();
                let metadata_key = d
                    .attribute("key")
                    .ok_or_else(|| format_err!("no key"))?
                    .trim_end_matches("/children")
                    .into();
                let title = d
                    .attribute("title")
                    .ok_or_else(|| format_err!("no title"))?
                    .into();
                let parent_key = d.attribute("parentKey").map(Into::into);
                let grandparent_key = d.attribute("grandparentKey").map(Into::into);
                let plex_metadata = PlexMetadata {
                    metadata_key,
                    object_type,
                    title,
                    parent_key,
                    grandparent_key,
                    ..PlexMetadata::default()
                };
                let plex_filename =
                    d.descendants()
                        .find_map(|n| n.attribute("file"))
                        .map(|f| PlexFilename {
                            metadata_key: plex_metadata.metadata_key.clone(),
                            filename: f.into(),
                            collection_id: None,
                        });
                Ok(Some((plex_metadata, plex_filename)))
            })
            .filter_map(Result::transpose)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use futures::TryStreamExt;
    use stdout_channel::StdoutChannel;

    use crate::{config::Config, imdb_episodes::ImdbEpisodes, movie_collection::MovieCollection, pgpool::PgPool, plex_events::PlexEvent};

    use super::PlexMetadata;

    #[tokio::test]
    #[ignore]
    async fn test_get_plex_filename() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;
        let events: Vec<_> = PlexEvent::get_events(&pool, None, None, None, None)
            .await?
            .try_collect()
            .await?;
        let event = events
            .into_iter()
            .find(|event| {
                event.metadata_key.is_some()
                    && event.metadata_key != Some("/library/metadata/22897".into())
            })
            .unwrap();
        println!("{:?}", event.metadata_key);
        let filename = event.get_filename(&config).await?;
        assert!(filename.filename.starts_with("/shares/"));
        Ok(())
    }

    #[test]
    fn test_extract_filename_from_xml() -> Result<(), Error> {
        let data = include_str!("../../tests/data/plex_metadata.xml");
        let output = PlexEvent::extract_filename_from_xml(&data)?;
        assert_eq!(
            output.as_str(),
            "/shares/seagate4000/Documents/movies/scifi/galaxy_quest.mp4"
        );
        Ok(())
    }

    #[test]
    fn test_extract_metadata_from_xml() -> Result<(), Error> {
        let data = include_str!("../../tests/data/plex_archer_s13_ep05.xml");
        let metadata = PlexMetadata::extract_metadata_from_xml(&data).unwrap();
        assert_eq!(
            metadata.title.as_str(),
            "RARBG - Archer.S13E04.WEBRip.x264-ION10"
        );
        assert_eq!(metadata.metadata_key.as_str(), "/library/metadata/27678");
        let data = include_str!("../../tests/data/plex_archer_s13.xml");
        let metadata = PlexMetadata::extract_metadata_from_xml(&data).unwrap();
        assert_eq!(metadata.title.as_str(), "Season 13");
        assert_eq!(metadata.metadata_key.as_str(), "/library/metadata/27233");
        let data = include_str!("../../tests/data/plex_archer.xml");
        let metadata = PlexMetadata::extract_metadata_from_xml(&data).unwrap();
        assert_eq!(metadata.title.as_str(), "Archer");
        assert_eq!(metadata.metadata_key.as_str(), "/library/metadata/19341");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_metadata_by_key() -> Result<(), Error> {
        let config = Config::with_config()?;
        let metadata = PlexMetadata::get_metadata_by_key(&config, "/library/metadata/27678")
            .await
            .unwrap();
        assert_eq!(
            metadata.title.as_str(),
            "RARBG - Archer.S13E04.WEBRip.x264-ION10"
        );
        Ok(())
    }

    #[test]
    fn test_extract_children_from_xml() -> Result<(), Error> {
        let data = include_str!("../../tests/data/archer_s13_children.xml");
        let children = PlexMetadata::extract_children_from_xml(&data)?;
        assert_eq!(children.len(), 8);
        let (metadata, filename) = &children[0];
        assert_eq!(metadata.metadata_key, "/library/metadata/27234");
        assert!(filename.is_some());
        assert_eq!(
            filename.as_ref().unwrap().filename,
            "/shares/dileptonnas/Documents/television/archer/season13/archer_s13_ep01.mp4"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_update_trakt_watched_from_plex_events() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;
        let stdout = StdoutChannel::new();
        let mc = MovieCollection::new(&config, &pool, &stdout);
        mc.fix_collection_episode_id().await?;
        let episodes: Vec<_> = ImdbEpisodes::get_episodes_not_recorded_in_trakt(&pool).await?.try_collect().await?;
        println!("{}", episodes.len());
        assert!(false);
        Ok(())
    }
}
