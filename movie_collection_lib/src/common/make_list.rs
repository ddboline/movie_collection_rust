use failure::Error;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{stdout, Write};
use std::path::Path;

use crate::common::config::Config;
use crate::common::utils::{get_video_runtime, map_result, walk_directory};

pub fn make_list() -> Result<(), Error> {
    let config = Config::with_config()?;

    let movies_dir = format!("{}/Documents/movies", config.home_dir);

    let path = Path::new(&movies_dir);

    let mut local_file_list: Vec<_> = path
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fname) => {
                let file_name = fname.file_name().into_string().unwrap();
                for suffix in &config.suffixes {
                    if file_name.ends_with(suffix) {
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

    let file_list: Vec<_> = config
        .movie_dirs
        .par_iter()
        .map(|d| walk_directory(&d, &local_file_list))
        .collect();

    let file_list: Vec<_> = map_result(file_list)?;

    let mut file_list: Vec<_> = file_list.into_iter().flatten().collect();

    file_list.sort();

    let file_map: HashMap<String, String> = file_list
        .iter()
        .map(|f| {
            let file_name = f.split('/').last().unwrap().to_string();
            (file_name, f.clone())
        })
        .collect();

    let result: Vec<_> = local_file_list
        .iter()
        .map(|f| {
            let full_path = match file_map.get(f) {
                Some(s) => s.clone(),
                None => "".to_string(),
            };
            format!("{} {}", f, full_path)
        })
        .collect();

    let stdout = stdout();

    for e in result {
        writeln!(stdout.lock(), "{}", e)?;
    }

    let result: Vec<_> = file_list
        .par_iter()
        .map(|f| {
            let timeval = get_video_runtime(f).unwrap_or_else(|_| "".to_string());

            format!("{} {}", timeval, f)
        })
        .collect();

    for e in result {
        writeln!(stdout.lock(), "{}", e)?;
    }

    Ok(())
}
