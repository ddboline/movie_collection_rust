use postgres_query::{query, FromSqlRow, query_dyn, Query, Parameter};
use serde::{Deserialize, Serialize};
use stack_string::{StackString, format_sstr};
use anyhow::{Error, format_err};
use time::OffsetDateTime;

use crate::date_time_wrapper::DateTimeWrapper;
use crate::pgpool::PgPool;

#[derive(FromSqlRow, Serialize, Deserialize)]
pub struct MusicCollection {
    pub id: i32,
    pub path: StackString,
    pub artist: Option<StackString>,
    pub album: Option<StackString>,
    pub title: Option<StackString>,
    pub last_modified: DateTimeWrapper,
}

impl MusicCollection {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_entries(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
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
                SELECT * FROM music_collection
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

    pub async fn get_by_id(pool: &PgPool, id: i32) -> Result<Option<Self>, Error> {
        let query = query!("SELECT * FROM music_collection WHERE id = $id", id=id);
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn insert(&self, pool: &PgPool) -> Result<u64, Error> {
        if Self::get_by_id(pool, self.id).await?.is_some() {
            return Err(format_err!("Cannot insert, object exists"));
        }
        let query = query!(
            "
                INSERT INTO music_collection (
                    id, path, artist, album, title, last_modified
                ) VALUES (
                    $id, $path, $artist, $album, $title, now()
                )
            ",
            id=self.id, path=self.path, artist=self.artist, album=self.album,
            title=self.title
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    pub async fn update(&self, pool: &PgPool) -> Result<u64, Error> {
        if Self::get_by_id(pool, self.id).await?.is_none() {
            return Err(format_err!("Cannot update, object doesn't exists"));
        }
        let query = query!(
            "
                UPDATE music_collection
                SET path=$path,artist=$artist,album=$album,title=$title,last_modified=now()
                WHERE id=$id
            ",
            id=self.id, path=self.path, artist=self.artist, album=self.album,
            title=self.title
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }
}