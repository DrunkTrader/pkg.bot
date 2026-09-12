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
-- $1: repology.repositories.metadata->>'family' values to filter by.
SELECT id::BIGINT                       AS id,
       name                             AS slug,
       COALESCE("desc", name)           AS name,
       metadata->>'family'              AS family,
       metadata->'repolinks'->0->>'url' AS homepage_url,
       last_updated::TEXT               AS revision
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
       l.url0 AS homepage_url,
       l.url2 AS repo_url,
       COALESCE(l.url9, l.url7) AS source_url,
       COALESCE(l.url5, l.url7, l.url9) AS package_url
FROM ranked p
-- Links are [type, link_id] pairs. Only resolve the types that map to a column.
LEFT JOIN LATERAL (
    SELECT (ARRAY_AGG(url ORDER BY ord) FILTER (WHERE kind = 0))[1] AS url0,
           (ARRAY_AGG(url ORDER BY ord) FILTER (WHERE kind = 2))[1] AS url2,
           (ARRAY_AGG(url ORDER BY ord) FILTER (WHERE kind = 5))[1] AS url5,
           (ARRAY_AGG(url ORDER BY ord) FILTER (WHERE kind = 7))[1] AS url7,
           (ARRAY_AGG(url ORDER BY ord) FILTER (WHERE kind = 9))[1] AS url9
    FROM (
        SELECT (e.link->>0)::INT AS kind, e.ord, k.url
        FROM JSON_ARRAY_ELEMENTS(
            CASE WHEN JSON_TYPEOF(p.links) = 'array' THEN p.links ELSE '[]'::JSON END
        ) WITH ORDINALITY AS e(link, ord)
        INNER JOIN repology.links k ON k.id = (e.link->>1)::INT
        WHERE (e.link->>0)::INT IN (0, 2, 5, 7, 9)
    ) x
) l ON TRUE
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
