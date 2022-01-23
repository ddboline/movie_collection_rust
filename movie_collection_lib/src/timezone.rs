use anyhow::{format_err, Error};
use chrono_tz::Tz;
use derive_more::Into;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{convert::TryFrom, ops::Deref, str::FromStr};

/// Direction in degrees
#[derive(Into, Debug, PartialEq, Copy, Clone, Eq, Serialize, Deserialize)]
#[serde(into = "StackString", try_from = "StackString")]
pub struct TimeZone(Tz);

impl Deref for TimeZone {
    type Target = Tz;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<TimeZone> for String {
    fn from(item: TimeZone) -> Self {
        item.0.name().into()
    }
}

impl From<TimeZone> for StackString {
    fn from(item: TimeZone) -> Self {
        item.0.name().into()
    }
}

impl FromStr for TimeZone {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse()
            .map(Self)
            .map_err(|e| format_err!("{e} is not a valid timezone"))
    }
}

impl TryFrom<&str> for TimeZone {
    type Error = Error;
    fn try_from(item: &str) -> Result<Self, Self::Error> {
        item.parse()
    }
}

impl TryFrom<StackString> for TimeZone {
    type Error = Error;
    fn try_from(item: StackString) -> Result<Self, Self::Error> {
        item.as_str().parse()
    }
}
