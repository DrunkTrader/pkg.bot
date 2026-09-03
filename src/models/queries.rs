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
    #[name = "get-repo"]
    pub get_repo: yesqlr::Query,
    #[name = "query-packages"]
    pub query_packages: yesqlr::Query,
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
}
