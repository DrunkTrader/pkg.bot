-- name: get-repos
SELECT * FROM repos ORDER BY score DESC, name;

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
-- $3: maintainer slug
-- $4: tags (JSON array)
-- $5: licenses (JSON array)
-- $6: platforms (JSON array)
-- $7: status
  AND ($3 = '' OR EXISTS (
      SELECT 1 FROM package_maintainers pm
      INNER JOIN maintainers mt ON mt.id = pm.maintainer_id
      WHERE pm.package_id = p.id AND mt.slug = $3
  ))
  AND ($4 = '[]' OR EXISTS (
      SELECT 1 FROM JSON_EACH(p.keywords) k, JSON_EACH($4) qk WHERE k.value = qk.value
  ))
  AND ($5 = '[]' OR EXISTS (
      SELECT 1 FROM JSON_EACH(p.licenses) l, JSON_EACH($5) ql WHERE l.value = ql.value
  ))
  AND ($6 = '[]' OR EXISTS (
      SELECT 1 FROM JSON_EACH(p.platforms) pf, JSON_EACH($6) qf WHERE pf.value = qf.value
  ))
  AND ($7 = '' OR p.status = $7)

-- name: get-packages
-- Get (browse) packages alphabetically with keyset pagination.Alphabetical listing with keyset pagination, driven off packages itself.
-- $1: repo_id
-- $2: unused here
-- $3-$7: see `filters`
-- $8: cursor name ('' for the first page)
-- $9: cursor id
-- $10: limit
SELECT p.*, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM packages p
INNER JOIN repos r ON r.id = p.repo_id
WHERE p.repo_id = $1
  AND (p.name, p.id) {CMP} ($8, $9)
{FILTERS}
ORDER BY p.name {DIR}, p.id {DIR}
LIMIT $10;

-- name: get-packages-by-license
-- $2: the license to page over. Params otherwise identical to get-packages.
SELECT p.*, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM package_licenses x
INNER JOIN packages p ON p.id = x.package_id
INNER JOIN repos r ON r.id = p.repo_id
WHERE x.repo_id = $1 AND x.license = $2
  AND (x.name, x.package_id) {CMP} ($8, $9)
{FILTERS}
ORDER BY x.name {DIR}, x.package_id {DIR}
LIMIT $10;

-- name: get-packages-by-keyword
-- $2: the keyword to page over. Params otherwise identical to get-packages.
SELECT p.*, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM package_keywords x
INNER JOIN packages p ON p.id = x.package_id
INNER JOIN repos r ON r.id = p.repo_id
WHERE x.repo_id = $1 AND x.keyword = $2
  AND (x.name, x.package_id) {CMP} ($8, $9)
{FILTERS}
ORDER BY x.name {DIR}, x.package_id {DIR}
LIMIT $10;

-- name: count-packages
-- Total for a filtered listing, counted no further than $8 so that the cost is
-- bounded; the caller reports anything beyond it as "$8+". Unfiltered listings
-- read repos.package_count instead.
-- $1-$7: see get-packages
-- $8: count limit
SELECT COUNT(*) FROM (
    SELECT 1 FROM packages p
    WHERE p.repo_id = $1
    {FILTERS}
    LIMIT $8
);

-- name: count-packages-by-license
SELECT COUNT(*) FROM (
    SELECT 1 FROM package_licenses x
    INNER JOIN packages p ON p.id = x.package_id
    WHERE x.repo_id = $1 AND x.license = $2
    {FILTERS}
    LIMIT $8
);

-- name: count-packages-by-keyword
SELECT COUNT(*) FROM (
    SELECT 1 FROM package_keywords x
    INNER JOIN packages p ON p.id = x.package_id
    WHERE x.repo_id = $1 AND x.keyword = $2
    {FILTERS}
    LIMIT $8
);

-- name: count-packages-by-maintainer
-- Driven off package_maintainers. The listing itself pages over packages, but an
-- exact count through the residual maintainer EXISTS would scan the whole repo.
-- The maintainer slug is $3, so $2 goes unused here.
SELECT COUNT(*) FROM (
    SELECT 1 FROM package_maintainers pm
    INNER JOIN maintainers mt ON mt.id = pm.maintainer_id
    INNER JOIN packages p ON p.id = pm.package_id
    WHERE p.repo_id = $1 AND mt.slug = $3
    {FILTERS}
    LIMIT $8
);

-- name: search-packages
-- Rank and retrieve packages matching an FTS expression. Only ever run with a
-- search term; empty-term listings go to get-packages.
-- $1: repo_id
-- $2: FTS expression
-- $3: lowercased query for exact slug match
-- $4: normalized query (lowercased, non-alphanumerics stripped) for exact/prefix name matching
-- $5: maintainer slug
-- $6: tags (JSON array)
-- $7: licenses (JSON array)
-- $8: platforms (JSON array)
-- $9: status
-- $10: normalized term for name/slug match.
-- $11: offset
-- $12: limit
--
-- Search logic:
-- Score the query against the package name. exact=200, prefix=+100, substring=+25, length ratio (shorter is better).
-- Then score against the package's []keywords.
--   +1000 if the query is exactly the slug.
--   +400 if slug == name
-- bm25 + boost for shorter slugs + stored static score which is manually set (eg: aur has a popularity score).
WITH matches AS (
    SELECT rowid AS pid, bm25(packages_fts) AS bm FROM packages_fts WHERE packages_fts MATCH $2
)
SELECT p.*, x.rank, x.total, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM (
    SELECT c.pid, c.name,
             -- Direct slug match is a big boost.
             (CASE WHEN c.slug_lc = $3 THEN 1000.0 ELSE 0.0 END)

           -- Higher score for a canonical package name match as opposed to the
           -- package name simply appearing in the description.
           -- In nixos, there is 'redis' vs. 'rubyPackages.redis' etc who have
           -- the same package name but aren't the same. This addresses those scenarios.
           + (CASE WHEN c.slug_lc = c.name_lc AND c.name_score > 0.0
                                                    THEN  400.0 ELSE 0.0 END)

           + c.name_score

           -- Small tiebreaker for equal scores.
           + (c.bm * -2.0)
           -- Rank shorter slugs higher. Shorter slugs/names are preferred over longer ones in results.
           + (200.0 / (10.0 + LENGTH(c.slug_lc)))
           + c.score AS rank,
           COUNT(*) OVER() AS total
    FROM (
        SELECT p.id AS pid, p.name, p.score, m.bm,
               LOWER(p.slug) AS slug_lc, LOWER(p.name) AS name_lc,

               -- Rank name matches higher than keyword matches. Keywords shouldn't
               -- add to the score of a package name (eg: 'neovim' as 'nvim' keyword,
               -- so will many other packages). Again, within that, shorter names are
               -- ranked higher.
               MAX(
                   (CASE WHEN p.name_norm = $4 THEN 200.0 ELSE 0.0 END)
                 + (CASE WHEN $4 <> '' AND p.name_norm LIKE $4 || '%' THEN 100.0 ELSE 0.0 END)
                 + (CASE WHEN $4 <> '' AND p.name_norm LIKE '%' || $4 || '%'
                         THEN 25.0 + 100.0 * LENGTH($4) / MAX(LENGTH(p.name_norm), 1)
                         ELSE 0.0 END),
                   COALESCE((
                       SELECT MAX(
                           (CASE WHEN LOWER(k.value) = $3 THEN 200.0 ELSE 0.0 END)
                         + (CASE WHEN LOWER(k.value) LIKE $3 || '%' THEN 100.0 ELSE 0.0 END)
                         + (CASE WHEN LOWER(k.value) LIKE '%' || $3 || '%' THEN 25.0 + 100.0 * LENGTH($3) / MAX(LENGTH(k.value), 1) ELSE 0.0 END))
                       FROM JSON_EACH(p.keywords) k WHERE $3 <> ''), 0.0)
               ) AS name_score
        FROM matches m
        INNER JOIN packages p ON p.id = m.pid
        WHERE p.repo_id = $1
          AND ($5 = '' OR EXISTS (
              SELECT 1 FROM package_maintainers pm
              INNER JOIN maintainers mt ON mt.id = pm.maintainer_id
              WHERE pm.package_id = p.id AND mt.slug = $5
          ))
          AND ($6 = '[]' OR EXISTS (
              SELECT 1 FROM JSON_EACH(p.keywords) k, JSON_EACH($6) qk WHERE k.value = qk.value
          ))
          AND ($7 = '[]' OR EXISTS (
              SELECT 1 FROM JSON_EACH(p.licenses) l, JSON_EACH($7) ql WHERE l.value = ql.value
          ))
          AND ($8 = '[]' OR EXISTS (
              SELECT 1 FROM JSON_EACH(p.platforms) pf, JSON_EACH($8) qf WHERE pf.value = qf.value
          ))
          AND ($9 = '' OR p.status = $9)
          AND ($10 = '' OR p.name_norm LIKE '%' || $10 || '%' OR LOWER(p.slug) LIKE '%' || $10 || '%')
    ) c
    ORDER BY rank DESC, c.name
    LIMIT $12 OFFSET $11
) x
INNER JOIN packages p ON p.id = x.pid
INNER JOIN repos r ON r.id = p.repo_id
ORDER BY x.rank DESC, p.name;
