use failure::{err_msg, Error};
use postgres::NoTls;
use r2d2::{Pool, PooledConnection};
use r2d2_postgres::PostgresConnectionManager;
use std::fmt;

#[derive(Clone)]
pub struct PgPool {
    pgurl: String,
    pool: Pool<PostgresConnectionManager<NoTls>>,
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PgPool {}", self.pgurl)
    }
}

impl PgPool {
    pub fn new(pgurl: &str) -> Self {
        let manager = PostgresConnectionManager::new(
            pgurl.parse().expect("Failed to parse DB connection"),
            NoTls,
        );
        Self {
            pgurl: pgurl.to_string(),
            pool: Pool::new(manager).expect("Failed to open DB connection"),
        }
    }

    pub fn get(&self) -> Result<PooledConnection<PostgresConnectionManager<NoTls>>, Error> {
        self.pool.get().map_err(err_msg)
    }
}
