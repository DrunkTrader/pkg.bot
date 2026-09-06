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
    #[name = "query-packages"]
    pub query_packages: yesqlr::Query,
    #[name = "cte-query-packages-fts"]
    pub cte_query_packages_fts: yesqlr::Query,
    #[name = "cte-query-packages-all"]
    pub cte_query_packages_all: yesqlr::Query,
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

    /// query-packages with the FTS `matches` CTE prepended, used when there is a
    /// search string.
    pub static ref query_packages_fts: String =
        format!("{}\n{}", q.cte_query_packages_fts.query, q.query_packages.query);

    /// query-packages with the full-scan `matches` CTE prepended, used when browsing.
    pub static ref query_packages_all: String =
        format!("{}\n{}", q.cte_query_packages_all.query, q.query_packages.query);
}

