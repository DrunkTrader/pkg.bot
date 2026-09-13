-- Queries for the Repology Postgres to SQLite importer.

-- name: set-pragmas
PRAGMA journal_mode       = WAL;
PRAGMA synchronous        = OFF;
PRAGMA foreign_keys       = OFF;      -- ids are resolved in memory before insert
PRAGMA busy_timeout       = 600000;   -- 10m; finalize holds the write lock for minutes
PRAGMA cache_size         = -1048576; -- 1GB
PRAGMA temp_store         = FILE;     -- the facet sort spills more rows than fit in RAM
PRAGMA wal_autocheckpoint = 10000;
PRAGMA analysis_limit     = 1000;     -- bound the closing PRAGMA optimize


-- name: pg-get-repos
-- $1: repology.repositories.metadata->>'family' values to filter by..
SELECT id::BIGINT                       AS id,
       name                             AS slug,
       COALESCE("desc", name)           AS name,
       metadata->>'family'              AS family,
       metadata->'repolinks'->0->>'url' AS homepage_url,
       COALESCE(metadata->'repolinks', '[]'::JSONB)::TEXT AS repolinks,
       (SELECT pl->>'url'
          FROM JSONB_ARRAY_ELEMENTS(COALESCE(metadata->'packagelinks', '[]'::JSONB)) pl
         WHERE (pl->>'type')::INT = 5
         ORDER BY (pl->>'priority')::INT, pl->>'url' LIMIT 1) AS pkg_url_template,
       -- Prefer the recipe file (9) over the directory listing (7).
       (SELECT pl->>'url'
          FROM JSONB_ARRAY_ELEMENTS(COALESCE(metadata->'packagelinks', '[]'::JSONB)) pl
         WHERE (pl->>'type')::INT IN (7, 9)
         ORDER BY (pl->>'type')::INT DESC, (pl->>'priority')::INT, pl->>'url' LIMIT 1) AS source_url_template
FROM repology.repositories
WHERE state = 'active' AND metadata->>'family' = ANY($1) ORDER BY id;


-- name: pg-check-libversion
-- Version ordering needs the `versiontext` extension.
SELECT TO_REGTYPE('versiontext') IS NOT NULL;


-- name: pg-get-maintainers
-- $1: the last maintainer read ('' on the first call).
SELECT maintainer FROM repology.maintainers WHERE num_packages > 0 AND maintainer > $1 ORDER BY maintainer LIMIT 25000;


-- name: pg-declare-packages
-- Cursor over the newest package per (repo, trackname)
-- $1: repology repository names to import.
DECLARE import_packages NO SCROLL CURSOR FOR
WITH ranked AS (
    SELECT id::BIGINT AS id, repo, family, srcname, binnames, trackname, visiblename,
           rawversion, version, maintainers, category, comment, licenses, effname, links,
           COALESCE(versionclass, 0)::INT AS versionclass,
           COALESCE(flags, 0)::INT AS flags,
           shadow,
           -- These are used by package url templates, eg: https://site.com/{subrepo}/{arch}/{name} etc.
           subrepo, arch,
           -- archs and subrepos only differ by build, so collect them across the group.
           ARRAY_AGG(arch) FILTER (WHERE arch IS NOT NULL) OVER w AS platforms,
           ARRAY_AGG(subrepo) FILTER (WHERE subrepo IS NOT NULL) OVER w AS subrepos,
           ROW_NUMBER() OVER (PARTITION BY repo, trackname ORDER BY
               CASE WHEN rawversion ~ '^[0-9]+:'
                    THEN SPLIT_PART(rawversion, ':', 1)::NUMERIC ELSE 0 END DESC,
               version::versiontext DESC,
               id) AS rank
    FROM repology.packages
    WHERE repo = ANY($1)
    WINDOW w AS (PARTITION BY repo, trackname)
)
SELECT p.id, p.repo, p.family, p.srcname, p.binnames, p.trackname, p.visiblename,
       p.rawversion, p.version, p.maintainers, p.category, p.comment, p.licenses,
       p.effname, p.versionclass, p.flags, p.shadow, p.platforms, p.subrepos,
       p.subrepo, p.arch,
       -- Links are [kind, link_id] pairs. Only need to get project homepage (kind=0)
       -- as the rest of the urls (package permalink, repo) are in repos.metadata->'packagelinks.
       (SELECT k.url
          FROM JSON_ARRAY_ELEMENTS(
                   CASE WHEN JSON_TYPEOF(p.links) = 'array' THEN p.links ELSE '[]'::JSON END
               ) WITH ORDINALITY AS e(link, ord)
          INNER JOIN repology.links k ON k.id = (e.link->>1)::INT
         WHERE (e.link->>0)::INT = 0
         ORDER BY e.ord LIMIT 1) AS homepage_url,
       TO_CHAR(COALESCE(rt.start_ts, rtv.start_ts) AT TIME ZONE 'UTC',
               'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
       TO_CHAR(COALESCE(rtv.start_ts, rt.start_ts) AT TIME ZONE 'UTC',
               'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
FROM ranked p
INNER JOIN repology.repositories r ON r.name = p.repo
LEFT JOIN repology.repo_tracks rt
       ON rt.repository_id = r.id AND rt.trackname = p.trackname
LEFT JOIN repology.repo_track_versions rtv
       ON rtv.repository_id = r.id AND rtv.trackname = p.trackname AND rtv.version = p.version
WHERE p.rank = 1;


-- name: pg-fetch-packages
FETCH FORWARD 25000 FROM import_packages;


-- name: update-counts
UPDATE repos SET package_count = (
    SELECT COUNT(*) FROM packages WHERE repo_id = repos.id
);
UPDATE maintainers SET package_count = (
    SELECT COUNT(*) FROM package_maintainers WHERE maintainer_id = maintainers.id
);
DELETE FROM maintainers WHERE package_count = 0;


-- name: build-facets
-- Build facets filter table.
INSERT OR IGNORE INTO package_facets (repo_id, kind, value, name, package_id)
    SELECT p.repo_id, 'license', l.value, p.name, p.id
    FROM packages p, JSON_EACH(p.licenses) l
    UNION ALL
    SELECT p.repo_id, 'tag', k.value, p.name, p.id
    FROM packages p, JSON_EACH(p.keywords) k
    UNION ALL
    SELECT p.repo_id, 'group', g.value, p.name, p.id
    FROM packages p, JSON_EACH(p."groups") g
    UNION ALL
    SELECT p.repo_id, 'status', p.status, p.name, p.id FROM packages p
    UNION ALL
    SELECT p.repo_id, 'is_nonfree', CAST(p.is_nonfree AS TEXT), p.name, p.id
    FROM packages p WHERE p.is_nonfree IS NOT NULL
    UNION ALL
    SELECT p.repo_id, 'maintainer', m.slug, p.name, p.id
    FROM packages p
    INNER JOIN package_maintainers pm ON pm.package_id = p.id
    INNER JOIN maintainers m ON m.id = pm.maintainer_id;

INSERT INTO facet_counts (repo_id, kind, value, package_count)
    SELECT repo_id, kind, value, COUNT(*) FROM package_facets GROUP BY 1, 2, 3;


-- name: build-fts
INSERT INTO packages_fts (rowid, identity, keywords, body, repo)
    SELECT id, identity_tokens, keyword_tokens, body_tokens, repo_id FROM packages;

INSERT INTO packages_fts (packages_fts) VALUES ('optimize');
