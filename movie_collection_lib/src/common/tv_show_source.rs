use failure::{err_msg, Error};
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
