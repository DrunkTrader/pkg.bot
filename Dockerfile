# Repology dumps require the postgresql-libversion extension, which is not
# shipped with the stock postgres image, so build it here.
FROM postgres:17

RUN printf '%s\n' 'CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;' \
    > /docker-entrypoint-initdb.d/01-extensions.sql

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        build-essential cmake git ca-certificates pkg-config \
        postgresql-server-dev-17; \
    git clone --depth 1 https://github.com/repology/libversion.git /tmp/libversion; \
    cmake -S /tmp/libversion -B /tmp/libversion/build -DCMAKE_BUILD_TYPE=Release; \
    cmake --build /tmp/libversion/build -j "$(nproc)"; \
    cmake --install /tmp/libversion/build; \
    ldconfig; \
    git clone --depth 1 https://github.com/repology/postgresql-libversion.git /tmp/pg-libversion; \
    make -C /tmp/pg-libversion -j "$(nproc)"; \
    make -C /tmp/pg-libversion install; \
    ldconfig; \
    ldd /usr/lib/postgresql/17/lib/libversion.so | grep -q 'libversion\.so\.1'; \
    rm -rf /tmp/libversion /tmp/pg-libversion; \
    apt-get purge -y --auto-remove build-essential cmake git pkg-config postgresql-server-dev-17; \
    rm -rf /var/lib/apt/lists/*
