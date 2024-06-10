use anyhow::{format_err, Error};
use stack_string::{format_sstr, StackString};
use std::fmt;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackType {
    Video,
    Audio,
    Subtitles,
}

impl TrackType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "subtitles" => Some(Self::Subtitles),
            _ => None,
        }
    }

    fn to_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Subtitles => "subtitles",
        }
    }
}

impl fmt::Display for TrackType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct MkvTrack {
    pub number: u64,
    pub track_type: Option<TrackType>,
    pub codec_id: StackString,
    pub name: StackString,
    pub language: StackString,
}

impl MkvTrack {
    fn parse_mkvinfo(input: &str) -> Vec<Self> {
        let mut tracks = Vec::new();
        let mut current_track: Option<Self> = None;

        for line in input.split('\n') {
            if line.starts_with("| + Track") {
                if let Some(track) = current_track.replace(Self::default()) {
                    tracks.push(track);
                }
            } else if line.starts_with("|  + Track number: ") {
                if let Some(entry) = line
                    .strip_prefix("|  + Track number: ")
                    .and_then(|l| l.split_ascii_whitespace().next())
                {
                    if let Some(track) = &mut current_track {
                        if let Ok(n) = entry.parse() {
                            track.number = n;
                        }
                    }
                }
            } else if line.starts_with("|  + Track type: ") {
                if let Some(entry) = line.strip_prefix("|  + Track type: ") {
                    if let Some(track_type) = TrackType::from_str(entry.trim()) {
                        if let Some(track) = &mut current_track {
                            track.track_type = Some(track_type);
                        }
                    }
                }
            } else if line.starts_with("|  + Codec ID: ") {
                if let Some(entry) = line.strip_prefix("|  + Codec ID: ") {
                    if let Some(track) = &mut current_track {
                        track.codec_id = entry.trim().into();
                    }
                }
            } else if line.starts_with("|  + Name: ") {
                if let Some(entry) = line.strip_prefix("|  + Name: ") {
                    if let Some(track) = &mut current_track {
                        track.name = entry.trim().into();
                    }
                }
            } else if line.starts_with("|  + Language: ") {
                if let Some(entry) = line.strip_prefix("|  + Language: ") {
                    if let Some(track) = &mut current_track {
                        track.language = entry.trim().into();
                    }
                }
            }
        }
        if let Some(track) = current_track.take() {
            tracks.push(track);
        }

        tracks
    }

    /// # Errors
    /// Return error if fpath doesn't end in mkv, or if output of mkvinfo is not
    /// utf8
    pub async fn get_subtitles_from_mkv(fpath: &str) -> Result<Vec<Self>, Error> {
        if !fpath.to_lowercase().ends_with(".mkv") {
            return Err(format_err!("Filename must end in mkv"));
        }

        let output = Command::new("/usr/bin/mkvinfo")
            .args([fpath])
            .output()
            .await?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(format_err!(
                "Process exited with error {:?}",
                output.status.code()
            ));
        }
        let output = StackString::from_utf8_vec(output.stdout)?;

        let tracks = Self::parse_mkvinfo(&output);

        Ok(tracks)
    }

    /// # Errors
    /// Return error if fpath doesn't end in mkv, or if output of mkvinfo is not
    /// utf8
    pub async fn extract_subtitles_from_mkv(fpath: &str, index: u64) -> Result<StackString, Error> {
        let fname = fpath
            .strip_suffix(".mkv")
            .ok_or_else(|| format_err!("Wrong suffix"))?;
        let srt_path = format_sstr!("{fname}.srt");
        if index < 1 {
            return Err(format_err!("Index must be greater than 0"));
        }
        let output = Command::new("/usr/bin/mkvextract")
            .args([fpath, "tracks", &format_sstr!("{}:{srt_path}", index - 1)])
            .output()
            .await?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(format_err!(
                "Process exited with error {:?}",
                output.status.code()
            ));
        }
        let output = StackString::from_utf8_vec(output.stdout)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use crate::mkv_utils::{MkvTrack, TrackType};

    #[test]
    fn test_parse_mkvinfo() -> Result<(), Error> {
        let input = include_str!("../../tests/data/mkvinfo_output.txt");
        let tracks = MkvTrack::parse_mkvinfo(input);
        assert_eq!(tracks.len(), 3);
        for track in &tracks {
            if track.track_type == Some(TrackType::Subtitles) {
                assert_eq!(track.number, 3);
                assert_eq!(track.codec_id.as_str(), "S_TEXT/UTF8");
                assert_eq!(track.name.as_str(), "Scarface - YIFY");
                assert_eq!(track.language.as_str(), "eng");
            }
        }
        Ok(())
    }
}
