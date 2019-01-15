version := "0.1.5"
release := "1"
uniq := $(shell head -c1000 /dev/urandom | sha512sum | head -c 12 ; echo ;)
cidfile := "/tmp/.tmp.docker.$(uniq)"
build_type := release

all:
	cp Dockerfile.ubuntu18.04 Dockerfile
	docker build -t movie_collection_rust/build_rust:ubuntu18.04 .
	rm Dockerfile

amazon:
	cp Dockerfile.amazonlinux2018.03 Dockerfile
	docker build -t movie_collection_rust/build_rust:amazonlinux2018.03 .
	rm Dockerfile

cleanup:
	docker rmi `docker images | python -c "import sys; print('\n'.join(l.split()[2] for l in sys.stdin if '<none>' in l))"`
	rm -rf /tmp/.tmp.docker.movie_collection_rust
	rm Dockerfile

package:
	docker run --cidfile $(cidfile) -v `pwd`/target:/movie_collection_rust/target movie_collection_rust/build_rust:ubuntu18.04 /movie_collection_rust/scripts/build_deb_docker.sh $(version) $(release)
	docker cp `cat $(cidfile)`:/movie_collection_rust/movie-collection-rust_$(version)-$(release)_amd64.deb .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

install:
	cp target/$(build_type)/make-list /usr/bin/make-list
	cp target/$(build_type)/remcom /usr/bin/remcom
	cp target/$(build_type)/transcode-avi /usr/bin/transcode-avi
	cp target/$(build_type)/run-encoding /usr/bin/run-encoding
	cp target/$(build_type)/parse-imdb /usr/bin/parse-imdb
	cp target/$(build_type)/make-collection /usr/bin/make-collection
	cp target/$(build_type)/make-queue /usr/bin/make-queue
	cp target/$(build_type)/movie-queue-http /usr/bin/movie-queue-http
	cp target/$(build_type)/trakt-app /usr/bin/trakt-app
	cp target/$(build_type)/find-new-episodes /usr/bin/find-new-episodes
	cp python/trakt_instance.py /usr/bin/trakt-flask-app
