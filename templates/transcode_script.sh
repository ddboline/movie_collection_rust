#!/bin/bash

nice -n 19 HandBrakeCLI -i INPUT_FILE -o OUTPUT_FILE --preset "Android 480p30" > ~/dvdrip/log/PREFIX_mp4.out 2>&1
mv OUTPUT_FILE ~/Documents/movies/
mv ~/dvdrip/log/PREFIX_mp4.out ~/tmp_avi/
