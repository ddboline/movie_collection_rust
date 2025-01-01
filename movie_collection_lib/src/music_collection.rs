use anyhow::{format_err, Error};
use futures::{future::try_join_all, Stream, TryStreamExt};
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, convert::TryInto, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    config::Config, date_time_wrapper::DateTimeWrapper, pgpool::PgPool, utils::walk_directory,
};

#[derive(FromSqlRow, Serialize, Deserialize, Debug)]
pub struct MusicCollection {
    pub id: Uuid,
    pub path: StackString,
    pub artist: Option<StackString>,
    pub album: Option<StackString>,
    pub title: Option<StackString>,
    pub last_modified: DateTimeWrapper,
}

impl Default for MusicCollection {
    fn default() -> Self {
        MusicCollection {
            id: Uuid::new_v4(),
            path: StackString::new(),
            artist: None,
            album: None,
            title: None,
            last_modified: DateTimeWrapper::now(),
        }
    }
}

impl MusicCollection {
    #[allow(clippy::ref_option)]
    fn get_music_collection_query<'a>(
        select_str: &'a str,
        order_str: &'a str,
        start_timestamp: &'a Option<OffsetDateTime>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Query<'a>, PqError> {
        let mut constraints = Vec::new();
        let mut bindings = Vec::new();
        if let Some(start_timestamp) = &start_timestamp {
            constraints.push("last_modified > $start_timestamp");
            bindings.push(("start_timestamp", start_timestamp as Parameter));
        }
        let where_str = if constraints.is_empty() {
            StackString::new()
        } else {
            format_sstr!("WHERE {}", constraints.join(" AND "))
        };
        let mut query = format_sstr!(
            "
                SELECT {select_str} FROM music_collection
                {where_str}
                {order_str}
            "
        );
        if let Some(offset) = &offset {
            query.push_str(&format_sstr!(" OFFSET {offset}"));
        }
        if let Some(limit) = &limit {
            query.push_str(&format_sstr!(" LIMIT {limit}"));
        }
        bindings.shrink_to_fit();
        debug!("query:\n{}", query);
        query_dyn!(&query, ..bindings)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_count(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
    ) -> Result<usize, Error> {
        #[derive(FromSqlRow)]
        struct Count {
            count: i64,
        }
        let query = Self::get_music_collection_query("count(*)", "", &start_timestamp, None, None)?;
        let conn = pool.get().await?;
        let count: Count = query.fetch_one(&conn).await?;
        Ok(count.count.try_into()?)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_entries(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = Self::get_music_collection_query(
            "*",
            "ORDER BY last_modified DESC",
            &start_timestamp,
            offset,
            limit,
        )?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Self>, Error> {
        let query = query!("SELECT * FROM music_collection WHERE id = $id", id = id);
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert(&self, pool: &PgPool) -> Result<u64, Error> {
        if Self::get_by_id(pool, self.id).await?.is_some() {
            return Err(format_err!("Cannot insert, object exists"));
        }
        let query = query!(
            "
                INSERT INTO music_collection (
                    id, path, artist, album, title
                ) VALUES (
                    $id, $path, $artist, $album, $title
                )
            ",
            id = self.id,
            path = self.path,
            artist = self.artist,
            album = self.album,
            title = self.title
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
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
            id = self.id,
            path = self.path,
            artist = self.artist,
            album = self.album,
            title = self.title
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn fix_plex_filename_music_collection_id(pool: &PgPool) -> Result<u64, Error> {
        let query = r"
            UPDATE plex_filename
            SET music_collection_id = (
                SELECT m.id
                FROM music_collection m
                WHERE m.path = replace(plex_filename.filename, '/shares/', '/media/')
            ),last_modified=now()
            WHERE music_collection_id IS NULL
        ";
        let rows = pool.get().await?.execute(query, &[]).await?;
        Ok(rows)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn make_collection(config: &Config, pool: &PgPool) -> Result<Vec<Self>, Error> {
        let music_dict: HashMap<StackString, Self> = Self::get_entries(pool, None, None, None)
            .await?
            .map_ok(|m| {
                let key: StackString = m.path.clone();
                (key, m)
            })
            .try_collect()
            .await?;
        let music_dict = Arc::new(music_dict);

        let music_list: Result<Vec<_>, Error> = config
            .music_dirs
            .iter()
            .filter(|d| d.exists())
            .map(|d| walk_directory(d, &["mp3"]))
            .collect();
        let music_list: Vec<_> = music_list?.into_iter().flatten().collect();
        if music_list.is_empty() {
            return Ok(Vec::new());
        }
        let futures = music_list.into_iter().map(|file| {
            let music_dict = music_dict.clone();
            async move {
                let p = file.to_string_lossy();
                if music_dict.contains_key(p.as_ref()) {
                    return Ok(None);
                }
                if !p.ends_with(".mp3") && !p.ends_with(".MP3") {
                    return Ok(None);
                }
                let mut iter = file.iter().skip(5);
                let artist = iter.next().map(|x| x.to_string_lossy().into());
                let album = iter.next().map(|x| x.to_string_lossy().into());
                let title = iter.next().map(|x| {
                    x.to_string_lossy()
                        .replace(".mp3", "")
                        .replace(".MP3", "")
                        .into()
                });
                let path = p.into();
                let id = Uuid::new_v4();
                let entry = MusicCollection {
                    id,
                    path,
                    artist,
                    album,
                    title,
                    last_modified: DateTimeWrapper::now(),
                };
                entry.insert(pool).await?;
                Ok(Some(entry))
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let results = results?;
        let count = results.iter().filter_map(Option::as_ref).count();
        println!("new entries {count}");
        let music_dict = Arc::try_unwrap(music_dict).unwrap_or_else(|_| HashMap::new());
        let all_entries: Vec<_> = results
            .into_iter()
            .flatten()
            .chain(music_dict.into_values())
            .collect();
        println!("all entries {}", all_entries.len());
        Ok(all_entries)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use crate::{config::Config, music_collection::MusicCollection, pgpool::PgPool};

    #[tokio::test]
    #[ignore]
    async fn test_make_music_collection() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;

        let result = MusicCollection::make_collection(&config, &pool).await?;
        assert!(!result.is_empty());
        assert!(result.len() > 0);
        Ok(())
    }
}
