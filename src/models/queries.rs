use lazy_static::lazy_static;
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
    #[name = "get-packages"]
    pub get_packages: yesqlr::Query,
    #[name = "get-packages-by-license"]
    pub get_packages_by_license: yesqlr::Query,
    #[name = "get-packages-by-keyword"]
    pub get_packages_by_keyword: yesqlr::Query,
    #[name = "count-packages"]
    pub count_packages: yesqlr::Query,
    #[name = "count-packages-by-license"]
    pub count_packages_by_license: yesqlr::Query,
    #[name = "count-packages-by-keyword"]
    pub count_packages_by_keyword: yesqlr::Query,
    #[name = "count-packages-by-maintainer"]
    pub count_packages_by_maintainer: yesqlr::Query,
    #[name = "search-packages"]
    pub search_packages: yesqlr::Query,
}

/// One driving source for a listing, pre-composed for paging either way.
pub struct Listing {
    pub next: String,
    pub prev: String,
    pub count: String,
}

lazy_static! {
    pub static ref schema: Schema = {
        let result = yesqlr::parse(SQL_SCHEMA).expect("error parsing schema.sql");
        Schema::try_from(result).expect("error reading SQL schema")
    };
    pub static ref q: Queries = {
        let result = yesqlr::parse(SQL_QUERIES).expect("error parsing queries.sql");
        Queries::try_from(result).expect("error reading SQL queries")
    };

    /// Paged over packages by name.
    pub static ref by_name: Listing = listing(&q.get_packages.query, &q.count_packages.query);

    /// Paged over one license's packages, seeking package_licenses.
    pub static ref by_license: Listing = listing(
        &q.get_packages_by_license.query,
        &q.count_packages_by_license.query
    );

    /// Paged over one keyword's packages, seeking package_keywords.
    pub static ref by_keyword: Listing = listing(
        &q.get_packages_by_keyword.query,
        &q.count_packages_by_keyword.query
    );

    /// Paged like by_name, but counted off package_maintainers.
    pub static ref by_maintainer: Listing = Listing {
        next: by_name.next.clone(),
        prev: by_name.prev.clone(),
        count: expand(&q.count_packages_by_maintainer.query, "", ""),
    };
}

fn listing(get: &str, count: &str) -> Listing {
    Listing {
        next: expand(get, ">", "ASC"),
        prev: expand(get, "<", "DESC"),
        count: expand(count, "", ""),
    }
}

fn expand(sql: &str, cmp: &str, dir: &str) -> String {
    sql.replace("{FILTERS}", &q.filters.query)
        .replace("{CMP}", cmp)
        .replace("{DIR}", dir)
}
