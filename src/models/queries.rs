use std::sync::LazyLock;
use yesqlr_macros::ScanQueries;

const SQL_SCHEMA: &[u8] = include_bytes!("../../static/sql/schema.sql");
const SQL_QUERIES: &[u8] = include_bytes!("../../static/sql/queries.sql");

/// Parsed SQL schema.
#[derive(Default, ScanQueries)]
pub struct Schema {
    pub pragma: yesqlr::Query,
    pub schema: yesqlr::Query,
}

/// Parsed SQL queries.
#[derive(Default, ScanQueries)]
pub struct Queries {
    #[name = "get-repos"]
    pub get_repos: yesqlr::Query,

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
