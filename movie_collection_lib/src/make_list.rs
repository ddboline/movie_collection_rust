use anyhow::Error;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{collections::HashMap, path::Path};

use crate::{
    config::Config,
    stdout_channel::StdoutChannel,
    utils::{get_video_runtime, walk_directory},
    stack_string::StackString,
};

pub fn make_list(stdout: &StdoutChannel) -> Result<(), Error> {
    let config = Config::with_config()?;
    let movies_dir = format!("{}/Documents/movies", config.home_dir);
    let path = Path::new(&movies_dir);

    let mut local_file_list: Vec<_> = path
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
        .map(|d| walk_directory(d.as_str(), &local_file_list))
        .collect();

    let mut file_list: Vec<_> = file_list?.into_iter().flatten().collect();

    file_list.sort();

    let file_map: HashMap<StackString, _> = file_list
        .iter()
        .map(|f| {
            let file_name = f.split('/').last().unwrap().into();
            (file_name, f)
        })
        .collect();

    let result: Vec<_> = local_file_list
        .iter()
        .map(|f| {
            let full_path = match file_map.get(f.as_str()) {
                Some(s) => s,
                None => "",
            };
            format!("{} {}", f, full_path)
        })
        .collect();

    for e in result {
        stdout.send(e.into())?;
    }

    let result: Vec<_> = file_list
        .par_iter()
        .map(|f| {
            let timeval = get_video_runtime(f).unwrap_or_else(|_| "".to_string());

            format!("{} {}", timeval, f)
        })
        .collect();

    for e in result {
        stdout.send(e.into())?;
    }

    Ok(())
}
