use sqlx::sqlite::SqlitePool;

use crate::models::{q, Package, PackageQuery};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("not found")]
    NotFound,
}

/// Manager handles all database operations and business logic.
pub struct Manager {
    db: SqlitePool,
}

impl Manager {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    /// Check whether a repo exists.
    pub async fn repo_exists(&self, id: i64) -> Result<(), Error> {
        sqlx::query(&q.get_repo.query)
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(Error::NotFound)?;

        Ok(())
    }

    /// Search packages in a repo. Reads offset/limit from the query.
    pub async fn query_packages(&self, pq: &PackageQuery) -> Result<(Vec<Package>, i64), Error> {
        // Column scoped FTS expressions, ANDed together. `query` matches the
        // descriptive fields whereas `name` is restricted to the package name.
        let fts: Vec<String> = [
            to_fts_query("name excerpt description", &pq.query),
            to_fts_query("name", &pq.name),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
        let fts = fts.join(" AND ");

        let results: Vec<Package> = sqlx::query_as(&q.query_packages.query)
            .bind(pq.repo_id)
            .bind(&fts)
            .bind(&pq.maintainer)
            .bind(to_json_list(&pq.tags))
            .bind(to_json_list(&pq.licenses))
            .bind(to_json_list(&pq.platform))
            .bind(&pq.status)
            .bind(pq.offset)
            .bind(pq.limit)
            .fetch_all(&self.db)
            .await?;

        let total = results.first().map(|p| p.total).unwrap_or(0);
        Ok((results, total))
    }
}

/// Convert a raw search string into an FTS5 expression scoped to the given columns.
/// Non-alphanumeric characters are dropped, matching the unicode61 tokenizer's
/// separators, and all terms are ANDed. Eg:
/// ("name", "foo-bar") => '{name} : ("foo" AND "bar")'
fn to_fts_query(cols: &str, query: &str) -> String {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.to_lowercase()))
        .collect();

    if terms.is_empty() {
        return String::new();
    }

    format!("{{{}}} : ({})", cols, terms.join(" AND "))
}

/// Convert repeated and/or comma separated values into a JSON array string for JSON_EACH().
fn to_json_list(vals: &[String]) -> String {
    let items: Vec<&str> = vals
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}
