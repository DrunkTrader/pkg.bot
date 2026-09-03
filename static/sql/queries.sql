-- name: get-repo
SELECT id FROM repos WHERE id = $1;

-- name: query-packages
-- Search packages in a repo with optional FTS and filters.
-- $1: repo_id, $2: FTS query, $3: maintainer slug, $4: tags (JSON array),
-- $5: licenses (JSON array), $6: platforms (JSON array), $7: status,
-- $8: offset, $9: limit
SELECT p.*, r.slug AS repo,
       (SELECT JSON_GROUP_ARRAY(JSON_OBJECT(
                   'slug', m.slug, 'handle', m.handle,
                   'name', m.name, 'email', m.email, 'url', m.url))
        FROM package_maintainers pm
        INNER JOIN maintainers m ON m.id = pm.maintainer_id
        WHERE pm.package_id = p.id) AS maintainers
FROM (
    SELECT p.*,
           -- Rank: BM25 (FTS match quality, negative = better) offset by the package score.
           CASE WHEN $2 = '' THEN 0.0 ELSE (
               SELECT bm25(packages_fts) FROM packages_fts
               WHERE packages_fts MATCH $2 AND rowid = p.id
           ) END - p.score AS rank,
           COUNT(*) OVER() AS total
    FROM packages p
    WHERE p.repo_id = $1
      AND ($2 = '' OR p.id IN (SELECT rowid FROM packages_fts WHERE packages_fts MATCH $2))
      AND ($3 = '' OR EXISTS (
          SELECT 1 FROM package_maintainers pm
          INNER JOIN maintainers m ON m.id = pm.maintainer_id
          WHERE pm.package_id = p.id AND m.slug = $3
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
    ORDER BY rank, p.name
    LIMIT $9 OFFSET $8
) p
INNER JOIN repos r ON r.id = p.repo_id
ORDER BY p.rank, p.name;
