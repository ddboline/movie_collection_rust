extern crate dotenv;
extern crate failure;
extern crate movie_collection_rust;
extern crate rayon;

use failure::Error;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;

use movie_collection_rust::config::Config;
use movie_collection_rust::utils::{get_video_runtime, map_result_vec, walk_directory};

fn make_list() -> Result<(), Error> {
    let config = Config::with_config();

    let movies_dir = format!("{}/Documents/movies", config.home_dir);

    let path = Path::new(&movies_dir);

    let local_file_list: Vec<_> = path
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

    let file_list: Vec<_> = config
        .movie_dirs
        .par_iter()
        .map(|d| walk_directory(&d, &local_file_list))
        .collect();

    let file_list: Vec<_> = map_result_vec(file_list)?.into_iter().flatten().collect();

    let file_map: HashMap<String, String> = file_list
        .iter()
        .map(|f| {
            let file_name = f.split('/').last().unwrap().to_string();
            (file_name, f.clone())
        })
        .collect();

    local_file_list
        .iter()
        .map(|f| {
            let full_path = match file_map.get(f) {
                Some(s) => s.clone(),
                None => "".to_string(),
            };
            println!("{} {}", f, full_path);
        })
        .for_each(drop);

    file_list
        .par_iter()
        .map(|f| {
            let timeval = get_video_runtime(f).unwrap_or_else(|_| "".to_string());

            println!("{} {}", timeval, f);
        })
        .for_each(drop);

    Ok(())
}

fn main() {
    make_list().unwrap();
}
