use bytes::BytesMut;
use failure::{err_msg, Error};
use postgres::types::{FromSql, IsNull, ToSql, Type};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

#[derive(Serialize, Deserialize, Clone, Debug, Eq)]
pub enum TvShowSource {
    #[serde(rename = "netflix")]
    Netflix,
    #[serde(rename = "hulu")]
    Hulu,
    #[serde(rename = "amazon")]
    Amazon,
    #[serde(rename = "all")]
    All,
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
            _ => Err(err_msg("Is not TvShowSource")),
        }
    }
}

impl Ord for TvShowSource {
    fn cmp(&self, other: &Self) -> Ordering {
        self.to_string().cmp(&other.to_string())
    }
}

impl PartialOrd for TvShowSource {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TvShowSource {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
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
