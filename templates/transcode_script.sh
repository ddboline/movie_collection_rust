#!/bin/bash

PREFIX="{{PREFIX}}"
INPUT_FILE="{{INPUT_FILE}}"
OUTPUT_FILE="{{OUTPUT_FILE}}"

nice -n 19 HandBrakeCLI -i $INPUT_FILE -o $OUTPUT_FILE --preset "Android 480p30" > ~/dvdrip/log/${PREFIX}_mp4.out 2>&1
mv $OUTPUT_FILE ~/Documents/movies/
mv ~/dvdrip/log/${PREFIX}_mp4.out ~/tmp_avi/
