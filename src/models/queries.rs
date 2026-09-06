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
    #[name = "get-packages"]
    pub get_packages: yesqlr::Query,
    #[name = "count-packages"]
    pub count_packages: yesqlr::Query,
    #[name = "search-packages"]
    pub search_packages: yesqlr::Query,
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

    /// get-packages paginates forward.
    pub static ref get_packages_next: String = keyset(">", "ASC");

    /// get-packages paginates backward.
    pub static ref get_packages_prev: String = keyset("<", "DESC");
}

fn keyset(cmp: &str, dir: &str) -> String {
    q.get_packages
        .query
        .replace("{CMP}", cmp)
        .replace("{DIR}", dir)
}
