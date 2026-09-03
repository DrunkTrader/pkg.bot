use serde::{Deserialize, Serialize};
use sqlx::{
    encode::IsNull,
    error::BoxDynError,
    sqlite::{SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef},
    Decode, Encode, FromRow, Sqlite, Type,
};

/// Package status bitflags stored in packages.status.
pub const STATUS_BROKEN: i64 = 1;
pub const STATUS_DEPRECATED: i64 = 2;
pub const STATUS_OUTDATED: i64 = 4;
pub const STATUS_DELETED: i64 = 8;

/// JSON array wrapper for SQLite TEXT columns storing JSON arrays.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JsonArray<T>(pub Vec<T>);

pub type StringArray = JsonArray<String>;

impl<T> From<Vec<T>> for JsonArray<T> {
    fn from(v: Vec<T>) -> Self {
        Self(v)
    }
}

impl<T> Type<Sqlite> for JsonArray<T> {
    fn type_info() -> SqliteTypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'q, T: Serialize> Encode<'q, Sqlite> for JsonArray<T> {
    fn encode_by_ref(&self, buf: &mut Vec<SqliteArgumentValue<'q>>) -> Result<IsNull, BoxDynError> {
        let json = serde_json::to_string(&self.0).unwrap_or_else(|_| "[]".to_string());
        <String as Encode<Sqlite>>::encode(json, buf)
    }
}

impl<'r, T: serde::de::DeserializeOwned> Decode<'r, Sqlite> for JsonArray<T> {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        let s = <&str as Decode<Sqlite>>::decode(value)?;
        if s.is_empty() {
            return Ok(Self(Vec::new()));
        }
        Ok(Self(serde_json::from_str(s)?))
    }
}

/// Maintainer of a package.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Maintainer {
    pub slug: String,
    pub handle: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub url: Option<String>,
}

/// JSON object stored as a String internally, but serialized as raw JSON.
/// Kind of emulates json.RawMessage behaviour in Go.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(from = "serde_json::Value")]
pub struct JsonString(pub String);

impl From<serde_json::Value> for JsonString {
    fn from(v: serde_json::Value) -> Self {
        Self(v.to_string())
    }
}

impl serde::Serialize for JsonString {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.0.is_empty() || self.0 == "{}" {
            // Serialize as empty object.
            use serde::ser::SerializeMap;
            serializer.serialize_map(Some(0))?.end()
        } else {
            // Parse the string as JSON and then serialize it.
            let value: serde_json::Value = serde_json::from_str(&self.0)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            value.serialize(serializer)
        }
    }
}

impl Type<Sqlite> for JsonString {
    fn type_info() -> SqliteTypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'q> Encode<'q, Sqlite> for JsonString {
    fn encode_by_ref(
        &self,
        buf: &mut Vec<SqliteArgumentValue<'q>>,
    ) -> std::result::Result<IsNull, BoxDynError> {
        let s = if self.0.is_empty() {
            "{}".to_string()
        } else {
            self.0.clone()
        };
        <String as Encode<Sqlite>>::encode(s, buf)
    }
}

impl<'r> Decode<'r, Sqlite> for JsonString {
    fn decode(value: SqliteValueRef<'r>) -> std::result::Result<Self, BoxDynError> {
        let s = <String as Decode<Sqlite>>::decode(value)?;
        Ok(Self(s))
    }
}

/// A package in a repository.
#[derive(Debug, Clone, Default, Serialize, FromRow)]
pub struct Package {
    /// Slug of the repo the package belongs to.
    pub repo: String,

    pub slug: String,
    pub name: String,
    pub name_norm: String,
    pub excerpt: Option<String>,
    pub description: Option<String>,
    pub pkg_base: Option<String>,

    pub version: Option<String>,
    pub version_norm: Option<String>,

    pub homepage_url: Option<String>,
    pub repo_url: Option<String>,
    pub source_url: Option<String>,

    pub licenses: StringArray,
    pub is_foss: Option<bool>,
    pub platforms: StringArray,

    pub groups: StringArray,
    pub keywords: StringArray,

    pub status: i64,
    pub download_bytes: Option<i64>,
    pub installed_bytes: Option<i64>,
    pub score: f64,

    pub meta: JsonString,
    pub hash: Option<String>,

    pub maintainers: JsonArray<Maintainer>,

    pub created_at: String,
    pub updated_at: String,
    pub built_at: String,

    // Pagination total (not serialized).
    #[sqlx(default)]
    #[serde(skip)]
    pub total: i64,
}

/// Package search query parameters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageQuery {
    /// FTS search across name, excerpt and description.
    #[serde(default)]
    pub query: String,

    /// FTS search restricted to name.
    #[serde(default)]
    pub name: String,

    /// Maintainer slug.
    #[serde(default)]
    pub maintainer: String,

    /// Keywords.
    #[serde(default)]
    pub tags: Vec<String>,

    /// License SPDX IDs.
    #[serde(default)]
    pub licenses: Vec<String>,

    /// Platforms.
    #[serde(default)]
    pub platform: Vec<String>,

    /// Comma separated status names (broken, deprecated, outdated, deleted).
    #[serde(default)]
    pub status: String,

    #[serde(default)]
    pub page: i32,

    #[serde(default)]
    pub per_page: i32,

    // Internal fields (not from HTTP query).
    #[serde(skip)]
    pub repo_id: i64,

    #[serde(skip)]
    pub offset: i32,

    #[serde(skip)]
    pub limit: i32,
}

/// Package search results.
#[derive(Debug, Clone, Serialize)]
pub struct PackageResults {
    pub packages: Vec<Package>,
    pub page: i32,
    pub per_page: i32,
    pub total: i64,
    pub total_pages: i32,
}

/// Application configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    pub app: AppConfig,
    pub db: DbConfig,

    #[serde(default)]
    pub api_results: ApiResultsConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub address: String,

    #[serde(default)]
    pub root_url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DbConfig {
    #[serde(default = "default_max_conns")]
    pub max_conns: u32,
}

fn default_max_conns() -> u32 {
    5
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiResultsConfig {
    #[serde(default = "default_api_per_page")]
    pub per_page: i32,
    #[serde(default = "default_api_max_per_page")]
    pub max_per_page: i32,
}

fn default_api_per_page() -> i32 {
    20
}

fn default_api_max_per_page() -> i32 {
    50
}

impl Default for ApiResultsConfig {
    fn default() -> Self {
        Self {
            per_page: default_api_per_page(),
            max_per_page: default_api_max_per_page(),
        }
    }
}
