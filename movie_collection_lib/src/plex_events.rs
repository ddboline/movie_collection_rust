use anyhow::Error;
use chrono::{DateTime, NaiveDateTime, Utc};
use postgres_query::{query, query_dyn, FromSqlRow, Parameter, Query};
use rweb::Schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    convert::{TryFrom, TryInto},
    net::Ipv4Addr,
};

use crate::{datetime_wrapper::DateTimeWrapper, pgpool::PgPool};

#[derive(FromSqlRow, Default, Debug, Serialize, Schema)]
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
}

impl TryFrom<WebhookPayload> for PlexEvent {
    type Error = Error;
    fn try_from(item: WebhookPayload) -> Result<Self, Self::Error> {
        fn dt_from_tm(x: u64) -> DateTimeWrapper {
            let dt = NaiveDateTime::from_timestamp(x as i64, 0);
            let dt = DateTime::from_utc(dt, Utc);
            dt.into()
        }

        serde_json::to_string(&item.event)
            .map(|event| {
                let event = event.trim_matches('"').into();
                Self {
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
                }
            })
            .map_err(Into::into)
    }
}

impl PlexEvent {
    pub fn get_from_payload(buf: &[u8]) -> Result<Self, Error> {
        let object: WebhookPayload = serde_json::from_slice(buf)?;
        object.try_into()
    }

    pub async fn get_events(
        pool: &PgPool,
        start_timestamp: Option<DateTime<Utc>>,
    ) -> Result<Vec<Self>, Error> {
        let mut query = format!("SELECT * FROM plex_event");
        let mut bindings = Vec::new();
        if let Some(start_timestamp) = &start_timestamp {
            query += " WHERE created_at > $start_timestamp";
            bindings.push(("start_timestamp", start_timestamp as Parameter));
        }
        let query: Query = query_dyn!(&query, ..bindings)?;
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub async fn write_event(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
            INSERT INTO plex_event (event, account, server, player_title, player_address, title,
                parent_title, grandparent_title, added_at, updated_at)
            VALUES ($event, $account, $server, $player_title, $player_address, $title,
                $parent_title, $grandparent_title, $added_at, $updated_at)",
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
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
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
}

#[derive(Serialize, Deserialize, Debug)]
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
