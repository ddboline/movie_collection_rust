use anyhow::{Error};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use movie_collection_lib::{
    movie_collection::{MovieCollection, MovieCollectionRow},
    movie_queue::{MovieQueueDB, MovieQueueResult, MovieQueueRow},
    pgpool::PgPool,
};

#[derive(Debug)]
pub struct MovieQueueRequest {
    pub patterns: Vec<StackString>,
}

impl MovieQueueRequest {
    pub async fn handle(
        self,
        pool: &PgPool,
    ) -> Result<(Vec<MovieQueueResult>, Vec<StackString>), Error> {
        let patterns: Vec<_> = self.patterns.iter().map(StackString::as_str).collect();
        let queue = MovieQueueDB::with_pool(pool)
            .print_movie_queue(&patterns)
            .await?;
        Ok((queue, self.patterns))
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueUpdateRequest {
    pub queue: Vec<MovieQueueRow>,
}

impl MovieQueueUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        let mq = MovieQueueDB::with_pool(pool);
        let mc = MovieCollection::with_pool(pool)?;
        for entry in &self.queue {
            let cidx = if let Some(i) = mc.get_collection_index(entry.path.as_ref()).await? {
                i
            } else {
                mc.insert_into_collection_by_idx(entry.collection_idx, entry.path.as_ref())
                    .await?;
                entry.collection_idx
            };
            assert_eq!(cidx, entry.collection_idx);
            mq.insert_into_queue_by_collection_idx(entry.idx, entry.collection_idx)
                .await?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieCollectionUpdateRequest {
    pub collection: Vec<MovieCollectionRow>,
}

impl MovieCollectionUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        let mc = MovieCollection::with_pool(pool)?;
        for entry in &self.collection {
            if let Some(cidx) = mc.get_collection_index(entry.path.as_ref()).await? {
                if cidx == entry.idx {
                    continue;
                }
                mc.remove_from_collection(entry.path.as_ref()).await?;
            };
            mc.insert_into_collection_by_idx(entry.idx, entry.path.as_ref())
                .await?;
        }
        Ok(())
    }
}
