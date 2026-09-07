use sqlx::sqlite::SqlitePool;

use crate::models::{
    by_keyword, by_license, by_maintainer, by_name, q, Cursor, Listing, Package, PackageQuery, Repo,
};

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

    /// Get alphabetical listing of a repo's packages.
    pub async fn get_packages(&self, pq: &PackageQuery) -> Result<(Vec<Package>, bool), Error> {
        let back = !pq.before.is_empty();
        let cur = Cursor::parse(if back { &pq.before } else { &pq.after });

        let (lst, drive) = driving_source(pq);

        // Fetch one extra row to detect whether there is another page after this.
        let mut packages: Vec<Package> = sqlx::query_as(if back { &lst.prev } else { &lst.next })
            .bind(pq.repo_id)
            .bind(drive)
            .bind(&pq.maintainer)
            .bind(to_json_list(&pq.tag))
            .bind(to_json_list(&pq.license))
            .bind(to_json_list(&pq.platform))
            .bind(&pq.status)
            .bind(&cur.name)
            .bind(cur.id)
            .bind(pq.limit + 1)
            .fetch_all(&self.db)
            .await?;

        let has_more = packages.len() > pq.limit as usize;
        packages.truncate(pq.limit as usize);
        if back {
            packages.reverse();
        }

        Ok((packages, has_more))
    }

    /// Total for a filtered listing, capped at [`MAX_COUNT`]. The bool reports
    /// whether the cap was hit, making the total a lower bound.
    pub async fn count_packages(&self, pq: &PackageQuery) -> Result<(i64, bool), Error> {
        let (lst, drive) = driving_source(pq);

        let n: i64 = sqlx::query_scalar(&lst.count)
            .bind(pq.repo_id)
            .bind(drive)
            .bind(&pq.maintainer)
            .bind(to_json_list(&pq.tag))
            .bind(to_json_list(&pq.license))
            .bind(to_json_list(&pq.platform))
            .bind(&pq.status)
            .bind(MAX_COUNT + 1)
            .fetch_one(&self.db)
            .await?;

        Ok((n.min(MAX_COUNT), n > MAX_COUNT))
    }

    pub async fn search_packages(&self, pq: &PackageQuery) -> Result<(Vec<Package>, i64), Error> {
        let (term, name_only) = pq.search();
        let raw = term.to_lowercase();
        let norm = norm_name(term);

        let packages: Vec<Package> = sqlx::query_as(&q.search_packages.query)
            .bind(pq.repo_id)
            .bind(to_fts_query(term))
            .bind(&raw)
            .bind(&norm)
            .bind(&pq.maintainer)
            .bind(to_json_list(&pq.tag))
            .bind(to_json_list(&pq.license))
            .bind(to_json_list(&pq.platform))
            .bind(&pq.status)
            .bind(if name_only { norm.as_str() } else { "" })
            .bind(pq.offset)
            .bind(pq.limit)
            .fetch_all(&self.db)
            .await?;

        let total = packages.first().map(|p| p.total).unwrap_or(0);
        Ok((packages, total))
    }
}

/// A filtered listing counts no further than this. An exact total means testing
/// every package in the repo, which no index can avoid for the JSON filters.
pub const MAX_COUNT: i64 = 1000;

const PREFIX_MIN_LEN: usize = 3;

/// Pick which table the listing pages over. A single license or tag can seek its
/// own side table, which carries the name sort key; several values can't, as the
/// index only orders within one value.
fn driving_source(pq: &PackageQuery) -> (&'static Listing, &str) {
    match (values(&pq.license).as_slice(), values(&pq.tag).as_slice()) {
        ([license], _) => (&by_license, license),
        (_, [keyword]) => (&by_keyword, keyword),
        _ if !pq.maintainer.is_empty() => (&by_maintainer, ""),
        _ => (&by_name, ""),
    }
}

/// Flatten repeated and/or comma separated filter values. Eg: ["a, b", "c"] => [a, b, c]
fn values(vals: &[String]) -> Vec<&str> {
    vals.iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .collect()
}

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

/// Filter values as a JSON array for JSON_EACH().
fn to_json_list(vals: &[String]) -> String {
    serde_json::to_string(&values(vals)).unwrap_or_else(|_| "[]".to_string())
}
