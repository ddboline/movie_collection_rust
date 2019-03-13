use failure::{err_msg, Error};
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
                TvShowSource::Netflix => "netflix",
                TvShowSource::Hulu => "hulu",
                TvShowSource::Amazon => "amazon",
                TvShowSource::All => "all",
            }
        )
    }
}

impl FromStr for TvShowSource {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "netflix" => Ok(TvShowSource::Netflix),
            "hulu" => Ok(TvShowSource::Hulu),
            "amazon" => Ok(TvShowSource::Amazon),
            "all" => Ok(TvShowSource::All),
            _ => Err(err_msg("Is not TvShowSource")),
        }
    }
}

impl Ord for TvShowSource {
    fn cmp(&self, other: &TvShowSource) -> Ordering {
        self.to_string().cmp(&other.to_string())
    }
}

impl PartialOrd for TvShowSource {
    fn partial_cmp(&self, other: &TvShowSource) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TvShowSource {
    fn eq(&self, other: &TvShowSource) -> bool {
        self.to_string() == other.to_string()
    }
}