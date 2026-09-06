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

-- name: cte-query-packages-fts
-- The `matches` CTE prepended to query-packages when there is a search string.
WITH matches AS (
    SELECT rowid AS pid, bm25(packages_fts) AS bm FROM packages_fts WHERE packages_fts MATCH $2
)

-- name: cte-query-packages-all
-- The `matches` CTE prepended to query-packages when there is no search string,
-- eg: browsing/directory listings.
WITH matches AS (
    SELECT id AS pid, 0.0 AS bm FROM packages WHERE repo_id = $1
)

-- name: query-packages
-- Rank and retrieve matched packages. One of the two `matches`
-- CTEs defined above is prepended to this query and executed during search.
-- $1: repo_id
-- $2: FTS expression
-- $3: 1 if there is a search query else 0 (skips ranking when browsing)
-- $4: lowercased query for exact slug match
-- $5: normalized query (lowercased, non-alphanumerics stripped) for exact/prefix name matching
-- $6: maintainer slug
-- $7: tags (JSON array)
-- $8: licenses (JSON array)
-- $9: platforms (JSON array)
-- $10: status
-- $11: offset
-- $12: limit
-- $13: normalized term for name/slug match.
--
-- Search logic:
-- Score the query against the package name. exact=200, prefix=+100, substring=+25, length ratio (shorter is better).
-- Then score against the package's []keywords.
--   +1000 if the query is exactly the slug.
--   +400 if slug == name
-- bm25 + boost for shorter slugs + stored static score which is manually set (eg: aur has a popularity score).
SELECT p.*, x.rank, x.total, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT('slug', m.slug, 'handle', m.handle,
                                            'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM (
    SELECT c.pid, c.name,
           CASE WHEN $3 = 0 THEN 0.0 ELSE
                  -- Direct slug match is a big boost.
                  (CASE WHEN c.slug_lc = $4 THEN 1000.0 ELSE 0.0 END)

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
                + c.score
           END AS rank,
           COUNT(*) OVER() AS total
    FROM (
        SELECT p.id AS pid, p.name, p.score, m.bm,
               LOWER(p.slug) AS slug_lc, LOWER(p.name) AS name_lc,

               -- Rank name matches higher than keyword matches. Keywords shouldn't
               -- add to the score of a package name (eg: 'neovim' as 'nvim' keyword,
               -- so will many other packages). Again, within that, shorter names are
               -- ranked higher.
               MAX(
                   (CASE WHEN p.name_norm = $5 THEN 200.0 ELSE 0.0 END)
                 + (CASE WHEN $5 <> '' AND p.name_norm LIKE $5 || '%' THEN 100.0 ELSE 0.0 END)
                 + (CASE WHEN $5 <> '' AND p.name_norm LIKE '%' || $5 || '%'
                         THEN 25.0 + 100.0 * LENGTH($5) / MAX(LENGTH(p.name_norm), 1)
                         ELSE 0.0 END),
                   COALESCE((
                       SELECT MAX(
                           (CASE WHEN LOWER(k.value) = $4 THEN 200.0 ELSE 0.0 END)
                         + (CASE WHEN LOWER(k.value) LIKE $4 || '%' THEN 100.0 ELSE 0.0 END)
                         + (CASE WHEN LOWER(k.value) LIKE '%' || $4 || '%' THEN 25.0 + 100.0 * LENGTH($4) / MAX(LENGTH(k.value), 1) ELSE 0.0 END))
                       FROM JSON_EACH(p.keywords) k WHERE $4 <> ''), 0.0)
               ) AS name_score
        FROM matches m
        INNER JOIN packages p ON p.id = m.pid
        WHERE p.repo_id = $1
          AND ($6 = '' OR EXISTS (
              SELECT 1 FROM package_maintainers pm
              INNER JOIN maintainers mt ON mt.id = pm.maintainer_id
              WHERE pm.package_id = p.id AND mt.slug = $6
          ))
          AND ($7 = '[]' OR EXISTS (
              SELECT 1 FROM JSON_EACH(p.keywords) k, JSON_EACH($7) qk WHERE k.value = qk.value
          ))
          AND ($8 = '[]' OR EXISTS (
              SELECT 1 FROM JSON_EACH(p.licenses) l, JSON_EACH($8) ql WHERE l.value = ql.value
          ))
          AND ($9 = '[]' OR EXISTS (
              SELECT 1 FROM JSON_EACH(p.platforms) pf, JSON_EACH($9) qf WHERE pf.value = qf.value
          ))
          AND ($10 = '' OR p.status = $10)
          AND ($13 = '' OR p.name_norm LIKE '%' || $13 || '%' OR LOWER(p.slug) LIKE '%' || $13 || '%')
    ) c
    ORDER BY rank DESC, c.name
    LIMIT $12 OFFSET $11
) x
INNER JOIN packages p ON p.id = x.pid
INNER JOIN repos r ON r.id = p.repo_id
ORDER BY x.rank DESC, p.name;
