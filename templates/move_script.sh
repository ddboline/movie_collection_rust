#!/bin/bash

cp ~/Documents/movies/SHOW.mp4 OUTNAME.mp4.new &
PID1=$!
mv FNAME ~/Documents/movies/BNAME.old
PID2=$1
wait $PID1 $PID2
mv ~/Documents/movies/BNAME.old ~/Documents/movies/BNAME
mv ONAME.mp4.new ONAME.mp4

make-queue rm FNAME
make-queue add ONAME.mp4
