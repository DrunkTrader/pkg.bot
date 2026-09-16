use std::sync::LazyLock;
use yesqlr_macros::ScanQueries;

const SQL_SCHEMA: &[u8] = include_bytes!("../../static/sql/schema.sql");
const SQL_QUERIES: &[u8] = include_bytes!("../../static/sql/queries.sql");
const SQL_REPOLOGY: &[u8] = include_bytes!("../../static/sql/repology.sql");
const SQL_IMPORT: &[u8] = include_bytes!("../../static/sql/import.sql");

/// Parsed SQL schema.
#[derive(Default, ScanQueries)]
pub struct Schema {
    pub pragma: yesqlr::Query,
    pub schema: yesqlr::Query,
}

/// Parsed SQL queries for the Repology restore.
#[derive(Default, ScanQueries)]
pub struct Repology {
    #[name = "set-timeouts"]
    pub set_timeouts: yesqlr::Query,

    pub schema: yesqlr::Query,

    #[name = "get-columns"]
    pub get_columns: yesqlr::Query,

    #[name = "create-indexes"]
    pub create_indexes: yesqlr::Query,
}

/// Parsed SQL queries for the `import` command.
#[derive(Default, ScanQueries)]
pub struct Import {
    #[name = "set-pragmas"]
    pub set_pragmas: yesqlr::Query,

    #[name = "pg-check-libversion"]
    pub pg_check_libversion: yesqlr::Query,

    #[name = "pg-get-repos"]
    pub pg_get_repos: yesqlr::Query,

    #[name = "pg-get-maintainers"]
    pub pg_get_maintainers: yesqlr::Query,

    #[name = "pg-declare-packages"]
    pub pg_declare_packages: yesqlr::Query,

    #[name = "pg-fetch-packages"]
    pub pg_fetch_packages: yesqlr::Query,

    #[name = "update-counts"]
    pub update_counts: yesqlr::Query,

    #[name = "build-facets"]
    pub build_facets: yesqlr::Query,

    #[name = "build-fts"]
    pub build_fts: yesqlr::Query,
}

/// Parsed SQL queries.
#[derive(Default, ScanQueries)]
pub struct Queries {
    #[name = "get-repo"]
    pub get_repo: yesqlr::Query,

    #[name = "get-repos"]
    pub get_repos: yesqlr::Query,

    #[name = "get-repos-grouped"]
    pub get_repos_grouped: yesqlr::Query,

    #[name = "get-package"]
    pub get_package: yesqlr::Query,

    #[name = "filters"]
    pub filters: yesqlr::Query,

    #[name = "pick-facet"]
    pub pick_facet: yesqlr::Query,

    #[name = "get-packages"]
    pub get_packages: yesqlr::Query,

    #[name = "get-packages-by-facet"]
    pub get_packages_by_facet: yesqlr::Query,

    #[name = "count-packages"]
    pub count_packages: yesqlr::Query,

    #[name = "count-packages-by-facet"]
    pub count_packages_by_facet: yesqlr::Query,

    #[name = "search-packages"]
    pub search_packages: yesqlr::Query,

    #[name = "get-licenses"]
    pub get_licenses: yesqlr::Query,

    #[name = "get-platforms"]
    pub get_platforms: yesqlr::Query,
}

/// A listing source, pre-composed for paging either way plus its capped count.
pub struct Listing {
    pub next: String,
    pub prev: String,
    pub count: String,
}

pub static SCHEMA: LazyLock<Schema> = LazyLock::new(|| {
    let result = yesqlr::parse(SQL_SCHEMA).expect("error parsing schema.sql");
    Schema::try_from(result).expect("error reading SQL schema")
});

pub static REPOLOGY: LazyLock<Repology> = LazyLock::new(|| {
    let result = yesqlr::parse(SQL_REPOLOGY).expect("error parsing repology.sql");
    Repology::try_from(result).expect("error reading SQL repology queries")
});

pub static IMPORT: LazyLock<Import> = LazyLock::new(|| {
    let result = yesqlr::parse(SQL_IMPORT).expect("error parsing import.sql");
    Import::try_from(result).expect("error reading SQL import queries")
});

pub static Q: LazyLock<Queries> = LazyLock::new(|| {
    let result = yesqlr::parse(SQL_QUERIES).expect("error parsing queries.sql");
    Queries::try_from(result).expect("error reading SQL queries")
});

/// Paged over packages by name.
pub static BY_NAME: LazyLock<Listing> =
    LazyLock::new(|| listing(&Q.get_packages.query, &Q.count_packages.query));

/// Browse packages matching one filter, sorted by name.
pub static BY_FACET: LazyLock<Listing> = LazyLock::new(|| {
    listing(
        &Q.get_packages_by_facet.query,
        &Q.count_packages_by_facet.query,
    )
});

/// Search query with the shared filters added.
pub static SEARCH_PACKAGES: LazyLock<String> =
    LazyLock::new(|| filters(&Q.search_packages.query, "$5", "$6"));

fn listing(get: &str, count: &str) -> Listing {
    Listing {
        next: keyset(get, ">", "ASC"),
        prev: keyset(get, "<", "DESC"),
        count: keyset(count, "", ""),
    }
}

fn keyset(sql: &str, cmp: &str, dir: &str) -> String {
    filters(sql, "$4", "$5")
        .replace("{CMP}", cmp)
        .replace("{DIR}", dir)
}

/// Add the shared filters using the given SQL parameter numbers.
fn filters(sql: &str, pairs: &str, platform: &str) -> String {
    let f = Q
        .filters
        .query
        .replace("{P}", pairs)
        .replace("{PF}", platform);

    sql.replace("{FILTERS}", &f)
}

/// Add version/date comparisons clauses.
pub fn comparisons(sql: &str, pq: &super::PackageQuery, first: usize) -> String {
    let clauses: Vec<_> = [
        ("p.version_norm", pq.version.as_str()),
        ("substr(p.updated_at, 1, 10)", pq.updated_at.as_str()),
    ]
    .iter()
    .enumerate()
    .map(|(i, (column, value))| {
        let param = first + i;
        if value.is_empty() {
            format!("AND ${param} IS NULL")
        } else {
            let (op, _) = super::get_comparator(value);
            format!("AND {column} {op} ${param}")
        }
    })
    .collect();
    sql.replace("{COMPARISONS}", &clauses.join("\n"))
}

#[cfg(test)]
mod comparison_tests {
    use super::*;
    use crate::models::{get_comparator, normalize_version, PackageQuery};

    #[tokio::test]
    async fn filter_dates_and_numeric_versions() {
        let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql("CREATE TABLE packages (repo_id INTEGER, version_norm TEXT, updated_at TEXT, platforms TEXT);
            CREATE TABLE package_facets (package_id INTEGER, kind TEXT, value TEXT);
            INSERT INTO packages VALUES
            (1, '00002.00000.00000', '2024-02-28T23:59:59Z', '[]'),
            (1, '00010.00000.00000', '2024-02-29T12:00:00Z', '[]'),
            (1, '00011.00000.00000', '2024-03-01T00:00:00Z', '[]'),
            (1, NULL, NULL, '[]');").execute(&db).await.unwrap();
        // Include id for the correlated facet filter.
        sqlx::query("ALTER TABLE packages ADD COLUMN id INTEGER")
            .execute(&db)
            .await
            .unwrap();
        for (version, date, expected) in [
            ("", "", 4_i64),
            ("2", "", 1),
            (">2", "", 2),
            ("<10", "", 1),
            ("", "2024-02-29", 1),
            ("", ">2024-02-29", 1),
            ("", "<2024-02-29", 1),
            (">2", "<2024-03-01", 1),
        ] {
            let mut pq = PackageQuery {
                version: version.into(),
                updated_at: date.into(),
                ..Default::default()
            };
            pq.validate().unwrap();
            let sql = comparisons(&BY_NAME.count, &pq, 7);
            let count: i64 = sqlx::query_scalar(&sql)
                .bind(1_i64)
                .bind("")
                .bind("")
                .bind("[]")
                .bind("")
                .bind(100_i64)
                .bind(normalize_version(get_comparator(&pq.version).1))
                .bind((!pq.updated_at.is_empty()).then(|| get_comparator(&pq.updated_at).1))
                .fetch_one(&db)
                .await
                .unwrap();
            assert_eq!(count, expected, "{version} {date}");
        }
    }
}
