use sqlx::sqlite::SqlitePool;

use crate::models::{q, query_packages_all, query_packages_fts, Package, PackageQuery, Repo};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("not found")]
    NotFound,
}

pub struct Manager {
    db: SqlitePool,
}

impl Manager {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    pub async fn get_repos(&self) -> Result<Vec<Repo>, Error> {
        Ok(sqlx::query_as(&q.get_repos.query)
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn get_package(&self, repo_id: i64, slug: &str) -> Result<Package, Error> {
        sqlx::query_as(&q.get_package.query)
            .bind(repo_id)
            .bind(slug)
            .fetch_optional(&self.db)
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn query_packages(&self, pq: &PackageQuery) -> Result<(Vec<Package>, i64), Error> {
        let (term, name_only) = pq.search();

        let fts = to_fts_query(term);
        let has_fts = !fts.is_empty();
        let raw = term.to_lowercase();
        let norm = norm_name(term);

        // Browsing uses a variant of the query that doesn't touch the FTS table at
        // all, so there is no MATCH to feed when there's nothing to search for.
        let sql: &str = if has_fts {
            &query_packages_fts
        } else {
            &query_packages_all
        };

        let results: Vec<Package> = sqlx::query_as(sql)
            .bind(pq.repo_id)
            .bind(&fts)
            .bind(has_fts as i32)
            .bind(&raw)
            .bind(&norm)
            .bind(&pq.maintainer)
            .bind(to_json_list(&pq.tags))
            .bind(to_json_list(&pq.licenses))
            .bind(to_json_list(&pq.platform))
            .bind(&pq.status)
            .bind(pq.offset)
            .bind(pq.limit)
            .bind(if name_only { norm.as_str() } else { "" })
            .fetch_all(&self.db)
            .await?;

        let total = results.first().map(|p| p.total).unwrap_or(0);
        Ok((results, total))
    }
}

const PREFIX_MIN_LEN: usize = 3;

/// Convert a raw search string into an FTS5 expression, ANDing all terms. The
/// last term becomes a prefix query so a query matches while it is being typed.
/// Eg: ("foo-bar") => '("foo" AND "bar" *)'
fn to_fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();

    if terms.is_empty() {
        return String::new();
    }

    let last = terms.len() - 1;
    let terms: Vec<String> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == last && t.chars().count() >= PREFIX_MIN_LEN {
                format!("\"{t}\" *")
            } else {
                format!("\"{t}\"")
            }
        })
        .collect();

    format!("({})", terms.join(" AND "))
}

/// Mirrors the transform the indexer applies to `packages.name_norm` so that
/// dash/underscore/case variants of a query match. Eg: "Foo_Bar-1" => "foobar1"
fn norm_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Convert repeated and/or comma separated values into a JSON array for JSON_EACH().
fn to_json_list(vals: &[String]) -> String {
    let items: Vec<&str> = vals
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}
