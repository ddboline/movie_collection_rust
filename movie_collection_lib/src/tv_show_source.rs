use anyhow::{format_err, Error};
use bytes::BytesMut;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, fmt, str::FromStr};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

#[derive(Serialize, Deserialize, Clone, Debug, Eq, Copy, PartialEq)]
pub enum TvShowSource {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "amazon")]
    Amazon,
    #[serde(rename = "hulu")]
    Hulu,
    #[serde(rename = "netflix")]
    Netflix,
}

impl TvShowSource {
    #[inline]
    fn ordering(self) -> u8 {
        match self {
            Self::All => 0,
            Self::Amazon => 1,
            Self::Hulu => 2,
            Self::Netflix => 3,
        }
    }
}

impl fmt::Display for TvShowSource {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Netflix => "netflix",
                Self::Hulu => "hulu",
                Self::Amazon => "amazon",
                Self::All => "all",
            }
        )
    }
}

impl FromStr for TvShowSource {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "netflix" => Ok(Self::Netflix),
            "hulu" => Ok(Self::Hulu),
            "amazon" => Ok(Self::Amazon),
            "all" => Ok(Self::All),
            _ => Err(format_err!("Is not TvShowSource")),
        }
    }
}

impl Ord for TvShowSource {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ordering().cmp(&other.ordering())
    }
}

impl PartialOrd for TvShowSource {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> FromSql<'a> for TvShowSource {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let s = String::from_sql(ty, raw)?.parse()?;
        Ok(s)
    }

    fn accepts(ty: &Type) -> bool {
        <String as FromSql>::accepts(ty)
    }
}

impl ToSql for TvShowSource {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <String as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}
