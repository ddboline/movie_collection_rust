use anyhow::Error;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::collections::HashMap;

use crate::{
    config::Config,
    stack_string::StackString,
    stdout_channel::StdoutChannel,
    utils::{get_video_runtime, walk_directory},
};

pub fn make_list(stdout: &StdoutChannel) -> Result<(), Error> {
    let config = Config::with_config()?;
    let movies_dir = config.home_dir.join("Documents").join("movies");

    let mut local_file_list: Vec<_> = movies_dir
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fname) => {
                let file_name = fname.file_name().into_string().unwrap();
                for suffix in &config.suffixes {
                    if file_name.ends_with(suffix.as_str()) {
                        return Some(file_name);
                    }
                }
                None
            }
            Err(_) => None,
        })
        .collect();

    if local_file_list.is_empty() {
        return Ok(());
    }

    local_file_list.sort();

    let file_list: Result<Vec<_>, Error> = config
        .movie_dirs
        .par_iter()
        .filter(|d| d.exists())
        .map(|d| walk_directory(&d, &local_file_list))
        .collect();

    let mut file_list: Vec<_> = file_list?.into_iter().flatten().collect();

    file_list.sort();

    let file_map: HashMap<StackString, _> = file_list
        .iter()
        .map(|f| {
            let file_name = f.file_name().unwrap().to_string_lossy().to_string().into();
            (file_name, f)
        })
        .collect();

    let result: Vec<_> = local_file_list
        .iter()
        .map(|f| {
            if let Some(full_path) = file_map.get(f.as_str()) {
                format!("{} {}", f, full_path.to_string_lossy())
            } else {
                f.to_string()
            }
        })
        .collect();

    for e in result {
        stdout.send(e.into())?;
    }

    let result: Vec<_> = file_list
        .par_iter()
        .map(|f| {
            let timeval = get_video_runtime(f).unwrap_or_else(|_| "".into());
            format!("{} {}", timeval, f.to_string_lossy())
        })
        .collect();

    for e in result {
        stdout.send(e.into())?;
    }

    Ok(())
}
