use anyhow::{format_err, Error};
use chrono::{DateTime, Local, NaiveDateTime, Utc};
use chrono_tz::Tz;
use log::info;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter, Query};
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    convert::{TryFrom, TryInto},
    net::Ipv4Addr,
    str::FromStr,
};

use crate::{config::Config, pgpool::PgPool};

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
    pub added_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub last_modified: DateTime<Utc>,
    pub metadata_type: Option<StackString>,
    pub section_type: Option<StackString>,
    pub section_title: Option<StackString>,
    pub metadata_key: Option<StackString>,
}

impl TryFrom<WebhookPayload> for PlexEvent {
    type Error = Error;
    fn try_from(item: WebhookPayload) -> Result<Self, Self::Error> {
        fn dt_from_tm(x: u64) -> DateTime<Utc> {
            let dt = NaiveDateTime::from_timestamp(x as i64, 0);
            DateTime::from_utc(dt, Utc)
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
            added_at: dt_from_tm(item.metadata.added_at),
            updated_at: item.metadata.updated_at.map(dt_from_tm),
            last_modified: Utc::now(),
            metadata_type: item.metadata.metadata_type,
            section_type: item.metadata.library_section_type,
            section_title: item.metadata.library_section_title,
            metadata_key: item.metadata.key,
        };
        Ok(payload)
    }
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
        start_timestamp: Option<DateTime<Utc>>,
        event_type: Option<PlexEventType>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Vec<Self>, Error> {
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
        query.fetch(&conn).await.map_err(Into::into)
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
    pub async fn get_event_http(
        pool: &PgPool,
        config: &Config,
        start_timestamp: Option<DateTime<Utc>>,
        event_type: Option<PlexEventType>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Vec<StackString>, Error> {
        #[derive(FromSqlRow)]
        struct EventOutput {
            event: StackString,
            metadata_type: Option<StackString>,
            section_title: Option<StackString>,
            title: StackString,
            parent_title: Option<StackString>,
            grandparent_title: Option<StackString>,
            filename: Option<StackString>,
            last_modified: DateTime<Utc>,
        }

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
        let query = format_sstr!(
            "
                SELECT a.event, a.metadata_type, a.section_title, a.title, a.parent_title,
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
        let output: Vec<EventOutput> = query.fetch(&conn).await?;
        let body = output
            .into_iter()
            .map(|event| {
                let last_modified = match config.default_time_zone {
                    Some(tz) => {
                        let tz: Tz = tz.into();
                        StackString::from_display(event.last_modified.with_timezone(&tz))
                    }
                    None => StackString::from_display(event.last_modified.with_timezone(&Local)),
                };
                format_sstr!(
                    r#"
                    <tr style="text-align; center;">
                    <td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>
                    </tr>
                "#,
                    last_modified,
                    event.event,
                    event.metadata_type.as_ref().map_or("", StackString::as_str),
                    event.section_title.as_ref().map_or("", StackString::as_str),
                    format_sstr!(
                        "{} {} {} {}",
                        event.title,
                        event.parent_title.as_ref().map_or("", StackString::as_str),
                        event
                            .grandparent_title
                            .as_ref()
                            .map_or("", StackString::as_str),
                        event.filename.as_ref().map_or("", StackString::as_str),
                    )
                )
            })
            .collect();
        Ok(body)
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
}

impl PlexFilename {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_filenames(
        pool: &PgPool,
        start_timestamp: Option<DateTime<Utc>>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Vec<Self>, Error> {
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
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_key(pool: &PgPool, key: &str) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM plex_filename WHERE metadata_key = $key",
            key = key,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "INSERT INTO plex_filename (metadata_key, filename)
            VALUES ($metadata_key, $filename)",
            metadata_key = self.metadata_key,
            filename = self.filename,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use crate::{config::Config, pgpool::PgPool, plex_events::PlexEvent};

    #[tokio::test]
    #[ignore]
    async fn test_get_plex_filename() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl);
        let event = PlexEvent::get_events(&pool, None, None, None, None)
            .await?
            .into_iter()
            .find(|event| event.metadata_key.is_some())
            .unwrap();
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
}
