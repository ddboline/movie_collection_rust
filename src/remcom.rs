extern crate dotenv;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::{err_msg, Error};
use std::fs::create_dir_all;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use movie_collection_rust::utils::{
    create_transcode_script, get_version_number, publish_transcode_job_to_queue, Config,
};

fn remcom() -> Result<(), Error> {
    let config = Config::new().with_config();

    let env_file = format!(
        "{}/.config/movie_collection_rust/config.env",
        config.home_dir
    );

    if Path::new("config.env").exists() {
        dotenv::from_filename("config.env").ok();
    } else if Path::new(&env_file).exists() {
        dotenv::from_path(&env_file).ok();
    } else if Path::new("config.env").exists() {
        dotenv::from_filename("config.env").ok();
    } else {
        dotenv::dotenv().ok();
    }

    let matches = App::new("Remcom")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Create script to do stuff")
        .arg(
            Arg::with_name("directory")
                .short("d")
                .long("directory")
                .value_name("DIRECTORY")
                .help("Directory"),
        )
        .arg(
            Arg::with_name("unwatched")
                .short("u")
                .long("unwatched")
                .value_name("UNWATCHED")
                .takes_value(false),
        )
        .arg(Arg::with_name("files").multiple(true))
        .get_matches();

    let unwatched = matches.is_present("unwatched");

    let directory = matches.value_of("directory").map(|d| d.to_string());

    if let Some(patterns) = matches.values_of("files") {
        let files: Vec<String> = patterns.map(|x| x.to_string()).collect();
        for file in files {
            let path = Path::new(&file);
            let ext = path.extension().unwrap().to_str().unwrap();

            if ext != "mp4" {
                match create_transcode_script(&config, &path) {
                    Ok(s) => {
                        println!("script {}", s);
                        publish_transcode_job_to_queue(
                            &s,
                            "transcode_work_queue",
                            "transcode_work_queue",
                        )
                        .expect("Publish to queue failed");
                    }
                    Err(e) => println!("error {}", e),
                }
            }

            match create_move_script(&config, directory.clone(), unwatched, &path) {
                Ok(s) => {
                    println!("script {}", s);
                    publish_transcode_job_to_queue(
                        &s,
                        "transcode_work_queue",
                        "transcode_work_queue",
                    )
                    .expect("Publish to queue failed");
                }
                Err(e) => println!("error {}", e),
            }
        }
    }
    Ok(())
}

fn create_move_script(
    config: &Config,
    directory: Option<String>,
    unwatched: bool,
    path: &Path,
) -> Result<String, Error> {
    let file = path.to_str().unwrap();
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let prefix = path.file_stem().unwrap().to_str().unwrap();
    let output_dir = if let Some(d) = directory {
        let d = format!("{}/Documents/movies/{}", config.preferred_dir, d);
        if !Path::new(&d).exists() {
            return Err(err_msg(format!("Directory {} does not exist", d)));
        }
        d
    } else if unwatched {
        let d = format!("{}/television/unwatched", config.preferred_dir);
        if !Path::new(&d).exists() {
            return Err(err_msg(format!("Directory {} does not exist", d)));
        }
        d
    } else {
        let file_stem: Vec<_> = path
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .split("_")
            .collect();
        let show = file_stem[..(file_stem.len() - 2)].join("_");
        let season: i64 = file_stem[(file_stem.len() - 2)]
            .replace("s", "")
            .parse()
            .unwrap_or(-1);
        let episode: i64 = file_stem[(file_stem.len() - 1)]
            .replace("ep", "")
            .parse()
            .unwrap_or(-1);

        if season == -1 || episode == -1 {
            panic!("Failed to parse show season {} episode {}", season, episode);
        }

        let d = format!(
            "{}/Documents/television/{}/season{}",
            config.preferred_dir, show, season
        );
        if !Path::new(&d).exists() {
            create_dir_all(&d)?;
        }
        d
    };
    let mp4_script = format!("{}/dvdrip/jobs/{}_copy.sh", config.home_dir, prefix);

    let script_str = include_str!("../templates/move_script.sh")
        .replace("SHOW", prefix)
        .replace("OUTNAME", &format!("{}/{}", output_dir, prefix))
        .replace("FNAME", file)
        .replace("BNAME", file_name)
        .replace("ONAME", &format!("{}/{}", output_dir, prefix));

    let mut f = File::create(mp4_script.clone())?;
    f.write_all(&script_str.into_bytes())?;

    println!("dir {} file {}", output_dir, file);
    Ok(mp4_script)
}

/*
cp ~/Documents/movies/SHOW.mp4 OUTNAME.mp4.new &
PID1=$!
mv FNAME ~/Documents/movies/BNAME.old
PID2=$1
wait $PID1 $PID2
mv ~/Documents/movies/BNAME.old ~/Documents/movies/BNAME
mv ONAME.mp4.new ONAME.mp4

make-queue rm FNAME
make-queue add ONAME.mp4

def _process(directory, unwatched, files):
    if directory:
        if not os.path.exists('%s/Documents/movies/%s' % (OUTPUT_DIR, directory)):
            raise Exception('bad directory')
        for fname in files:
            input_filename = '%s/%s' % (INPUT_DIR, fname)
            mp4_filename = fname.replace('.avi', '.mp4')
            prefix = os.path.basename(input_filename)
            for suffix in '.avi', '.mp4', '.mkv':
                prefix = prefix.replace(suffix, '')
            mp4_script = '%s/dvdrip/jobs/%s_mp4.sh' % (HOMEDIR, prefix)
            output_directory = '%s/Documents/movies/%s' % (OUTPUT_DIR, directory)

            if not os.path.exists(input_filename):
                continue

            if fname.endswith('.mp4'):
                mp4_script = '%s/dvdrip/jobs/%s_mp4.sh' % (HOMEDIR, prefix)
                with open(mp4_script, 'w') as inpf:
                    inpf.write('#!/bin/bash\n')
                    inpf.write('cp %s/Documents/movies/%s %s/Documents/movies/'
                               '%s/\n' % (HOMEDIR, fname, OUTPUT_DIR, directory))
                    inpf.write('%s/bin/make_queue add %s/Documents/movies/%s/%s\n' %
                               (HOMEDIR, OUTPUT_DIR, directory, mp4_filename))
                publish_transcode_job_to_queue(mp4_script)
                continue

            remcom_main(input_filename, output_directory, 0, 0)
            if not os.path.exists(mp4_script):
                print('something bad happened %s' % input_filename)
                exit(0)
            with open(mp4_script, 'a') as inpf:
                inpf.write('cp %s/Documents/movies/%s %s/Documents/movies/'
                           '%s/\n' % (HOMEDIR, mp4_filename, OUTPUT_DIR, directory))
                inpf.write('mv %s/Documents/movies/%s/%s %s/'
                           'Documents/movies/\n' % (OUTPUT_DIR, directory, fname, HOMEDIR))
                inpf.write('rm %s/tmp_avi/%s_0.mpg\n' % (HOMEDIR, prefix))
                inpf.write('%s/bin/make_queue add %s/Documents/movies/%s/%s\n' %
                           (HOMEDIR, OUTPUT_DIR, directory, mp4_filename))
            publish_transcode_job_to_queue(mp4_script)
    elif unwatched:
        for fname in files:
            input_filename = '%s/%s' % (INPUT_DIR, fname)
            mp4_filename = fname.replace('.avi', '.mp4')
            prefix = input_filename.split('/')[-1]
            for suffix in '.avi', '.mp4', '.mkv':
                prefix = prefix.replace(suffix, '')
            mp4_script = '%s/dvdrip/jobs/%s_mp4.sh' % (HOMEDIR, prefix)
            output_dir = '%s/television/unwatched' % OUTPUT_DIR

            if not os.path.exists(input_filename):
                continue
            if fname.endswith('.mp4'):
                with open(mp4_script, 'w') as inpf:
                    inpf.write('#!/bin/bash\n')
                    inpf.write('cp %s/Documents/movies/%s %s/'
                               'television/unwatched/\n' % (HOMEDIR, fname, OUTPUT_DIR))
                    inpf.write('%s/bin/make_queue add %s/%s\n' % (HOMEDIR, output_dir,
                                                                  mp4_filename))
                publish_transcode_job_to_queue(mp4_script)
                continue

            remcom_main(input_filename, output_dir, 0, 0)
            if not os.path.exists(mp4_script):
                print('something bad happened %s' % input_filename)
                exit(0)
            with open(mp4_script, 'a') as inpf:
                inpf.write('cp %s/Documents/movies/%s %s/'
                           'television/unwatched/\n' % (HOMEDIR, mp4_filename, OUTPUT_DIR))
                inpf.write('mv %s/Documents/movies/%s %s/Documents/movies/%s\n' %
                           (HOMEDIR, fname, HOMEDIR, fname.replace('.avi', '.old.avi')))
                inpf.write('mv %s/television/unwatched/%s %s/'
                           'Documents/movies/\n' % (OUTPUT_DIR, fname, HOMEDIR))
                inpf.write('rm %s/tmp_avi/%s_0.mpg\n' % (HOMEDIR, prefix))
                inpf.write('%s/bin/make_queue add %s/%s\n' % (HOMEDIR, output_dir, mp4_filename))
            publish_transcode_job_to_queue(mp4_script)
    else:
        for fname in files:
            tmp = fname.split('.')[0].split('_')
            try:
                show = '_'.join(tmp[:-2])
                season = int(tmp[-2].strip('s'))
                episode = int(tmp[-1].strip('ep'))
            except ValueError:
                _process(directory=directory, unwatched=True, files=[fname])
                continue

            prefix = '%s_s%02d_ep%02d' % (show, season, episode)
            output_dir = '%s/Documents/television/%s/season%d' % (OUTPUT_DIR, show, season)
            inavifile = '%s/Documents/movies/%s.avi' % (HOMEDIR, prefix)
            mp4_script = '%s/dvdrip/jobs/%s_mp4.sh' % (HOMEDIR, prefix)
            avifile = '%s/%s.avi' % (output_dir, prefix)
            mp4file = '%s/Documents/movies/%s.mp4' % (HOMEDIR, prefix)
            mp4file_final = '%s/%s.mp4' % (output_dir, prefix)

            if fname.endswith('.mp4'):
                with open(mp4_script, 'w') as inpf:
                    inpf.write('#!/bin/bash\n')
                    inpf.write('mkdir -p %s\n' % output_dir)
                    inpf.write('cp %s %s\n' % (mp4file, mp4file_final))
                    inpf.write('%s/bin/make_queue add %s\n' % (HOMEDIR, mp4file_final))
                publish_transcode_job_to_queue(mp4_script)
                continue

            remcom_main(inavifile, output_dir, 0, 0)
            if not os.path.exists(mp4_script):
                print('something bad happened %s' % prefix)
                exit(0)
            with open(mp4_script, 'a') as inpf:
                inpf.write('mkdir -p %s\n' % output_dir)
                inpf.write('cp %s %s\n' % (mp4file, mp4file_final))
                inpf.write('mv %s %s/Documents/movies/%s.old.avi\n' % (avifile, HOMEDIR, prefix))
                inpf.write('rm %s/tmp_avi/%s_0.mpg\n' % (HOMEDIR, prefix))
                inpf.write('%s/bin/make_queue add %s\n' % (HOMEDIR, mp4file_final))
            publish_transcode_job_to_queue(mp4_script)
*/

fn main() {
    remcom().unwrap();
}
