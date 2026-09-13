-- name: get-repos
-- {ORDER_BY}, {ORDER} are substited as literals.
SELECT * FROM repos
WHERE ($1 = '' OR family = $1)
  AND ($2 = '' OR IFNULL(distro, '') = $2)
  AND ($3 = '' OR manager = $3)
ORDER BY {ORDER_BY} COLLATE NOCASE {ORDER}, name COLLATE NOCASE;

-- name: get-package
-- $1: repo_id
-- $2: package slug
SELECT p.*, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM packages p
INNER JOIN repos r ON r.id = p.repo_id
WHERE p.repo_id = $1 AND p.slug = $2;

-- name: filters
-- Shared filters, inserted at {FILTERS} in browsing and search queries.
-- {P} = facets as a JSON array of {"k": kind, "v": value}
-- {PF} = platform, checked against packages.platforms
  AND NOT EXISTS (
      SELECT 1 FROM JSON_EACH({P}) req
      WHERE NOT EXISTS (
          SELECT 1 FROM package_facets g
          WHERE g.package_id = p.id
            AND g.kind = req.value ->> 'k' AND g.value = req.value ->> 'v'
      )
  )
  AND ({PF} = '' OR EXISTS (
      SELECT 1 FROM JSON_EACH(p.platforms) pf WHERE pf.value = {PF}
  ))

-- name: pick-facet
-- Start with the filter matching the fewest packages to reduce the work.
-- $1: repo_id
-- $2: filter facets (JSON array)
SELECT c.kind, c.value, COUNT(*) OVER() AS matched
FROM JSON_EACH($2) req
INNER JOIN facet_counts c
   ON c.repo_id = $1 AND c.kind = req.value ->> 'k' AND c.value = req.value ->> 'v'
ORDER BY c.package_count, c.kind
LIMIT 1;

-- name: get-packages
-- Browse packages by name when no facet filter is set.
-- {CMP} and {DIR} set the page direction.
-- $1: repo_id
-- $2, $3: unused here; the starting filter for get-packages-by-facet
-- $4-$5: facet JSON and platform
-- $6: cursor name ('' for the first page)
-- $7: cursor id
-- $8: limit
SELECT p.*, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM packages p
INNER JOIN repos r ON r.id = p.repo_id
WHERE p.repo_id = $1
  AND (p.name, p.id) {CMP} ($6, $7)
{FILTERS}
ORDER BY p.name {DIR}, p.id {DIR}
LIMIT $8;

-- name: get-packages-by-facet
-- Browse packages matching one filter. $4 contains the remaining filters.
-- $2: starting filter kind
-- $3: starting filter value
-- Params otherwise identical to get-packages.
SELECT p.*, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM package_facets f
INNER JOIN packages p ON p.id = f.package_id
INNER JOIN repos r ON r.id = p.repo_id
WHERE f.repo_id = $1 AND f.kind = $2 AND f.value = $3
  AND (f.name, f.package_id) {CMP} ($6, $7)
{FILTERS}
ORDER BY f.name {DIR}, f.package_id {DIR}
LIMIT $8;

-- name: count-packages
-- Count matches up to $6 when no saved count can be used.
-- $1-$5: see get-packages
-- $6: count limit
SELECT COUNT(*) FROM (
    SELECT 1 FROM packages p WHERE p.repo_id = $1 {FILTERS} LIMIT $6
);

-- name: count-packages-by-facet
SELECT COUNT(*) FROM (
    SELECT 1 FROM package_facets f
    INNER JOIN packages p ON p.id = f.package_id
    WHERE f.repo_id = $1 AND f.kind = $2 AND f.value = $3
    {FILTERS} LIMIT $6
);

-- name: search-packages
-- $1: repo_id (0 for all); $2: scoped FTS expression
-- $3: lowercased query; $4: normalized query for exact name comparison
-- $5-$6: facet JSON and platform; $7: offset; $8: limit
-- Field weights: identity=10, keywords=5, body=1, repo=0.
-- Favor exact slug matches, then name and keyword matches.
-- Shorter slugs get a larger share of the name and text match scores.
WITH matches AS (
    SELECT rowid AS pid, -bm25(packages_fts, 10.0, 5.0, 1.0, 0.0) AS relevance,
           -bm25(packages_fts, 10.0, 5.0, 0.0, 0.0) AS identity_relevance
    FROM packages_fts WHERE packages_fts MATCH $2
)
SELECT p.*, x.rank, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM (
    SELECT p.id AS pid, p.name,
           (CASE WHEN LOWER(p.slug) = $3 THEN 1000.0 ELSE 0.0 END)
           + MIN(1.0, 1.0 * LENGTH($3) / MAX(LENGTH(p.slug), 1)) * (
               CASE WHEN m.identity_relevance > 0.0 THEN 100.0 ELSE 0.0 END
               + CASE WHEN $4 <> '' AND p.name_norm = $4 THEN 25.0 ELSE 0.0 END
               + 100.0 * m.relevance / (1.0 + m.relevance)
           ) + p.score AS rank
    FROM matches m
    INNER JOIN packages p ON p.id = m.pid
    WHERE ($1 = 0 OR p.repo_id = $1)
{FILTERS}
    ORDER BY rank DESC, p.name, p.id
    LIMIT $8 OFFSET $7
) x
INNER JOIN packages p ON p.id = x.pid
INNER JOIN repos r ON r.id = p.repo_id
ORDER BY x.rank DESC, p.name, p.id;

-- name: rebuild-facets
-- Rebuild filter rows and counts for $1 (repo_id)
-- Run in the same transaction as package updates.
DELETE FROM package_facets WHERE repo_id = $1;
DELETE FROM facet_counts WHERE repo_id = $1;

-- Read package payloads once and build facets.
WITH selected AS MATERIALIZED (
    SELECT repo_id, id, name, licenses, keywords, "groups", status, is_nonfree
    FROM packages WHERE repo_id = $1
)
INSERT OR IGNORE INTO package_facets (repo_id, kind, value, name, package_id)
    SELECT p.repo_id, 'license', l.value, p.name, p.id
    FROM selected p, JSON_EACH(p.licenses) l
    UNION ALL
    SELECT p.repo_id, 'tag', k.value, p.name, p.id
    FROM selected p, JSON_EACH(p.keywords) k
    UNION ALL
    SELECT p.repo_id, 'group', g.value, p.name, p.id
    FROM selected p, JSON_EACH(p."groups") g
    UNION ALL
    SELECT p.repo_id, 'status', p.status, p.name, p.id FROM selected p
    UNION ALL
    SELECT p.repo_id, 'is_nonfree', CAST(p.is_nonfree AS TEXT), p.name, p.id
    FROM selected p WHERE p.is_nonfree IS NOT NULL
    UNION ALL
    SELECT p.repo_id, 'maintainer', mt.slug, p.name, p.id
    FROM selected p
    CROSS JOIN package_maintainers pm ON pm.package_id = p.id
    CROSS JOIN maintainers mt ON mt.id = pm.maintainer_id
    ORDER BY 1, 2, 3, 4, 5;

INSERT INTO facet_counts (repo_id, kind, value, package_count)
    SELECT repo_id, kind, value, COUNT(*) FROM package_facets WHERE repo_id = $1 GROUP BY 1, 2, 3;
