#!/bin/bash

cp ~/Documents/movies/SHOW.mp4 OUTNAME.mp4.new &
PID1=$!
if [ -e FNAME ]; then
    mv FNAME ~/Documents/movies/BNAME.old &
    PID2=$!
    wait $PID2
fi
wait $PID1
if [ -e ~/Documents/movies/BNAME.old ]; then
    mv ~/Documents/movies/BNAME.old ~/Documents/movies/BNAME
fi
mv ONAME.mp4.new ONAME.mp4

make-queue -r FNAME
make-queue -a ONAME.mp4
