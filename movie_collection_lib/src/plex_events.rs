use anyhow::{format_err, Error};
use chrono::{DateTime, Local, NaiveDateTime, Utc};
use chrono_tz::Tz;
use log::info;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter, Query};
use rweb::Schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    convert::{TryFrom, TryInto},
    net::Ipv4Addr,
    str::FromStr,
};

use crate::{config::Config, datetime_wrapper::DateTimeWrapper, pgpool::PgPool};

#[derive(FromSqlRow, Default, Debug, Serialize, Deserialize, Schema)]
pub struct PlexEvent {
    pub event: StackString,
    pub account: StackString,
    pub server: StackString,
    pub player_title: StackString,
    pub player_address: StackString,
    pub title: Option<StackString>,
    pub parent_title: Option<StackString>,
    pub grandparent_title: Option<StackString>,
    pub added_at: Option<DateTimeWrapper>,
    pub updated_at: Option<DateTimeWrapper>,
    pub created_at: Option<DateTimeWrapper>,
    pub last_modified: Option<DateTimeWrapper>,
    pub metadata_type: Option<StackString>,
    pub section_type: Option<StackString>,
    pub section_title: Option<StackString>,
}

impl TryFrom<WebhookPayload> for PlexEvent {
    type Error = Error;
    fn try_from(item: WebhookPayload) -> Result<Self, Self::Error> {
        fn dt_from_tm(x: u64) -> DateTimeWrapper {
            let dt = NaiveDateTime::from_timestamp(x as i64, 0);
            let dt = DateTime::from_utc(dt, Utc);
            dt.into()
        }
        let event = item.event.to_str().into();
        let payload = Self {
            event,
            account: item.account.title,
            server: item.server.title,
            player_title: item.player.title,
            player_address: item.player.public_address.to_string().into(),
            title: item.metadata.title,
            parent_title: item.metadata.parent_title,
            grandparent_title: item.metadata.grandparent_title,
            added_at: item.metadata.added_at.map(dt_from_tm),
            updated_at: item.metadata.updated_at.map(dt_from_tm),
            created_at: Some(Utc::now().into()),
            last_modified: Some(Utc::now().into()),
            metadata_type: item.metadata.metadata_type,
            section_type: item.metadata.library_section_type,
            section_title: item.metadata.library_section_title,
        };
        Ok(payload)
    }
}

impl PlexEvent {
    pub fn get_from_payload(buf: &[u8]) -> Result<Self, Error> {
        let object: WebhookPayload = serde_json::from_slice(buf)?;
        info!("{:#?}", object);
        object.try_into()
    }

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
            constraints.push("created_at > $start_timestamp");
            bindings.push(("start_timestamp", start_timestamp as Parameter));
        }
        let event_type = event_type.map(|s| s.to_str().to_string());
        if let Some(event_type) = &event_type {
            constraints.push("event = $event");
            bindings.push(("event", event_type as Parameter));
        }
        let query = format!(
            "
                SELECT * FROM plex_event
                {where}
                ORDER BY created_at DESC
                {limit}
                {offset}
            ",
            where = if constraints.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", constraints.join(" AND "))
            },
            limit = if let Some(limit) = limit {
                format!("LIMIT {}", limit)
            } else {
                String::new()
            },
            offset = if let Some(offset) = offset {
                format!("OFFSET {}", offset)
            } else {
                String::new()
            }
        );
        let query: Query = query_dyn!(&query, ..bindings)?;
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub async fn write_event(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
            INSERT INTO plex_event (
                event, account, server, player_title, player_address, title, parent_title,
                grandparent_title, added_at, updated_at, created_at, last_modified,
                metadata_type, section_type, section_title
            )
            VALUES (
                $event, $account, $server, $player_title, $player_address, $title,
                $parent_title, $grandparent_title, $added_at, $updated_at, $created_at,
                $last_modified, $metadata_type, $section_type, $section_title
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
            created_at = self.created_at,
            last_modified = self.last_modified,
            metadata_type = self.metadata_type,
            section_type = self.section_type,
            section_title = self.section_title,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }

    pub fn event_http(&self, config: &Config) -> StackString {
        let created_at =
            self.created_at
                .map_or(String::new(), |created_at| match config.default_time_zone {
                    Some(tz) => {
                        let tz: Tz = tz.into();
                        created_at.with_timezone(&tz).to_string()
                    }
                    None => created_at.with_timezone(&Local).to_string(),
                });
        format!(
            r#"<tr style="text-align; center;"><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            created_at,
            self.event,
            self.metadata_type.as_ref().map_or("", |s| s.as_str()),
            self.section_title.as_ref().map_or("", |s| s.as_str()),
            format!(
                "{} {} {}",
                self.title.as_ref().map_or("", |s| s.as_str()),
                self.parent_title.as_ref().map_or("", |s| s.as_str()),
                self.grandparent_title.as_ref().map_or("", |s| s.as_str()),
            )
        ).into()
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
    #[serde(rename = "publicAddress")]
    pub public_address: Ipv4Addr,
    pub title: StackString,
    pub uuid: StackString,
}

#[derive(Deserialize, Debug)]
pub struct Metadata {
    #[serde(rename = "type")]
    pub metadata_type: Option<StackString>,
    pub title: Option<StackString>,
    #[serde(rename = "parentTitle")]
    pub parent_title: Option<StackString>,
    #[serde(rename = "grandparentTitle")]
    pub grandparent_title: Option<StackString>,
    pub summary: Option<StackString>,
    pub rating: Option<f64>,
    #[serde(rename = "ratingCount")]
    pub rating_count: Option<u64>,
    pub key: Option<StackString>,
    #[serde(rename = "parentKey")]
    pub parent_key: Option<StackString>,
    #[serde(rename = "grandparentKey")]
    pub grandparent_key: Option<StackString>,
    #[serde(rename = "addedAt")]
    pub added_at: Option<u64>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<u64>,
    #[serde(rename = "librarySectionType")]
    pub library_section_type: Option<StackString>,
    #[serde(rename = "librarySectionTitle")]
    pub library_section_title: Option<StackString>,
}

#[derive(Serialize, Deserialize, Debug, Schema, Clone, Copy)]
pub enum PlexEventType {
    #[serde(rename = "library.on.deck")]
    LibraryOnDeck,
    #[serde(rename = "library.new")]
    LibraryNew,
    #[serde(rename = "media.pause")]
    MediaPause,
    #[serde(rename = "media.play")]
    MediaPlay,
    #[serde(rename = "media.rate")]
    MediaRate,
    #[serde(rename = "media.resume")]
    MediaResume,
    #[serde(rename = "media.scrobble")]
    MediaScrobble,
    #[serde(rename = "media.stop")]
    MediaStop,
    #[serde(rename = "admin.database.backup")]
    AdminDatabaseBackup,
    #[serde(rename = "admin.database.corrupted")]
    AdminDatabaseCorrupted,
    #[serde(rename = "device.new")]
    DeviceNew,
    #[serde(rename = "playback.started")]
    PlaybackStarted,
}

impl PlexEventType {
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
    #[serde(rename = "Account")]
    pub account: Account,
    #[serde(rename = "Server")]
    pub server: Server,
    #[serde(rename = "Player")]
    pub player: Player,
    #[serde(rename = "Metadata")]
    pub metadata: Metadata,
}
