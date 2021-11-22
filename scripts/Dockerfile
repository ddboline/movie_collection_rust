FROM rust:alpine3.14 as buildstage

RUN apk add musl-dev && mkdir -p /build/movie_collection_rust

ADD Cargo.toml /build/movie_collection_rust/
COPY src /build/movie_collection_rust/src
COPY movie_collection_http /build/movie_collection_rust/movie_collection_http
COPY movie_collection_lib /build/movie_collection_rust/movie_collection_lib
COPY transcode_lib /build/movie_collection_rust/transcode_lib
COPY migrations /build/movie_collection_rust/migrations
COPY templates /build/movie_collection_rust/templates

RUN mkdir -p /movie_collection_rust && \
    cd /build/movie_collection_rust && \
    cargo install --path=. --root=/movie_collection_rust

FROM alpine:3.14

COPY --from=buildstage /movie_collection_rust/bin/* /usr/bin/

RUN apk add openssh-client

STOPSIGNAL SIGTERM

EXPOSE 8042

CMD ["/usr/bin/aws-app-http"]
