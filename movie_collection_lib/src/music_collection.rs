use anyhow::{format_err, Error};
use futures::future::try_join_all;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter, Query};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    config::Config, date_time_wrapper::DateTimeWrapper, pgpool::PgPool, utils::walk_directory,
};

#[derive(FromSqlRow, Serialize, Deserialize)]
pub struct MusicCollection {
    pub id: Uuid,
    pub path: StackString,
    pub artist: Option<StackString>,
    pub album: Option<StackString>,
    pub title: Option<StackString>,
    pub last_modified: DateTimeWrapper,
}

impl MusicCollection {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_count(pool: &PgPool) -> Result<i64, Error> {
        #[derive(FromSqlRow)]
        struct Count {
            count: i64,
        }
        let query = query!("SELECT count(*) AS count FROM music_collection");
        let conn = pool.get().await?;
        let count: Count = query.fetch_one(&conn).await?;
        Ok(count.count)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_max_id(pool: &PgPool) -> Result<Option<i32>, Error> {
        #[derive(FromSqlRow)]
        struct MaxId {
            max_id: i32,
        }
        if Self::get_count(pool).await? == 0 {
            return Ok(None);
        }
        let query = query!("SELECT max(id) as max_id FROM music_collection");
        let conn = pool.get().await?;
        let max_id: MaxId = query.fetch_one(&conn).await?;
        Ok(Some(max_id.max_id))
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_entries(
        pool: &PgPool,
        start_timestamp: Option<OffsetDateTime>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Vec<Self>, Error> {
        if Self::get_count(pool).await? == 0 {
            return Ok(Vec::new());
        }
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
                    id, path, artist, album, title, last_modified
                ) VALUES (
                    $id, $path, $artist, $album, $title, now()
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
        let query = r#"
            UPDATE plex_filename
            SET music_collection_id = (
                SELECT m.id
                FROM music_collection m
                WHERE m.path = replace(plex_filename.filename, '/shares/', '/media/')
            )
            WHERE music_collection_id IS NULL
        "#;
        let rows = pool.get().await?.execute(query, &[]).await?;
        Ok(rows)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn make_collection(config: &Config, pool: &PgPool) -> Result<Vec<Self>, Error> {
        let music_dict: HashMap<StackString, Self> = Self::get_entries(pool, None, None, None)
            .await?
            .into_iter()
            .map(|m| {
                let key: StackString = m.path.clone();
                (key, m)
            })
            .collect();
        let music_dict = Arc::new(music_dict);

        let music_list: Result<Vec<_>, Error> = config
            .music_dirs
            .par_iter()
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
            .chain(music_dict.into_iter().map(|(_, v)| v))
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
        let pool = PgPool::new(&config.pgurl);

        let result = MusicCollection::make_collection(&config, &pool).await?;
        assert!(!result.is_empty());
        assert!(result.len() > 0);
        Ok(())
    }
}
