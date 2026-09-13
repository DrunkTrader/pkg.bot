use serde::{Deserialize, Serialize};
use sqlx::{
    encode::IsNull,
    error::BoxDynError,
    sqlite::{SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef},
    Decode, Encode, FromRow, Sqlite, Type,
};

/// Status of a package (eg: active, deleted etc.).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum PackageStatus {
    #[default]
    Active,
    Broken,
    Outdated,
    Deleted,
}

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

/// Sort order.
pub const ORDER_ASC: &str = "asc";
pub const ORDER_DESC: &str = "desc";

/// Columns the repo listings can be sorted by.
pub const REPO_SORT_FIELDS: [&str; 6] = [
    "name",
    "distro",
    "manager",
    "package_count",
    "num_maintainers",
    "synced_at",
];

/// `?order_by=&order=` on a listing page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sort {
    pub order_by: String,
    pub order: String,
}

impl Sort {
    /// Sort ascending by `field`.
    pub fn asc(field: &str) -> Self {
        Self {
            order_by: field.to_string(),
            order: ORDER_ASC.to_string(),
        }
    }

    /// Validate sort fields.
    pub fn clamp(mut self, fields: &[&str], default: &str) -> Self {
        if !fields.contains(&self.order_by.as_str()) {
            self.order_by = default.to_string();
        }
        self.order = if self.order.eq_ignore_ascii_case(ORDER_DESC) {
            ORDER_DESC
        } else {
            ORDER_ASC
        }
        .to_string();

        self
    }
}

/// A package repository (a distro's repo, channel or branch).
#[derive(Debug, Clone, Default, Serialize, FromRow)]
pub struct Repo {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub family: String,
    pub manager: String,
    pub distro: Option<String>,
    pub branch: Option<String>,

    pub homepage_url: Option<String>,
    pub pkg_url_template: Option<String>,
    pub source_url_template: Option<String>,
    pub meta: JsonString,
    pub brand_color: Option<String>,

    pub score: f64,
    pub package_count: i64,
    pub num_packages: i32,
    pub num_maintainers: i32,

    pub created_at: String,
    pub updated_at: String,
    pub synced_at: String,
}

/// A package in a repository.
#[derive(Debug, Clone, Default, Serialize, FromRow)]
pub struct Package {
    #[serde(skip)]
    pub id: i64,

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

    pub licenses: StringArray,
    pub is_nonfree: Option<bool>,
    pub platforms: StringArray,

    pub groups: StringArray,
    pub keywords: StringArray,

    pub status: PackageStatus,
    pub download_bytes: Option<i64>,
    pub installed_bytes: Option<i64>,
    pub score: f64,

    pub meta: JsonString,
    pub hash: Option<String>,

    pub maintainers: JsonArray<Maintainer>,

    pub created_at: String,
    pub updated_at: String,
}

/// Package search query parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageQuery {
    /// Search across every indexed field. Mutually exclusive with `name`.
    #[serde(default)]
    pub q: String,

    /// Search restricted to the package name and slug. Mutually exclusive with `q`.
    #[serde(default)]
    pub name: String,

    /// Filters. Each takes a single value, eg: `license=MIT&status=broken`.
    #[serde(default)]
    pub maintainer: String,

    #[serde(default)]
    pub tag: String,

    #[serde(default)]
    pub license: String,

    #[serde(default)]
    pub group: String,

    #[serde(default)]
    pub platform: String,

    #[serde(default)]
    pub status: String,

    /// Accepts "true"/"false" or "1"/"0"; validate() converts these to "1"/"0".
    #[serde(default)]
    pub is_nonfree: String,

    /// Keyset pagination fields.
    #[serde(default)]
    pub after: String,

    #[serde(default)]
    pub before: String,

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

impl PackageQuery {
    /// Reject combined `q` and `name` searches and convert `is_nonfree` to "1"/"0".
    pub fn validate(&mut self) -> Result<(), &'static str> {
        if !self.q.trim().is_empty() && !self.name.trim().is_empty() {
            return Err("q and name cannot be used together");
        }

        self.is_nonfree = match self.is_nonfree.trim() {
            "" => String::new(),
            "1" | "true" => "1".into(),
            "0" | "false" => "0".into(),
            _ => return Err("is_nonfree must be true or false"),
        };

        Ok(())
    }

    /// Return nonempty filters as (kind, value) pairs. Platform is checked separately.
    pub fn facets(&self) -> Vec<(&'static str, &str)> {
        [
            ("license", self.license.trim()),
            ("tag", self.tag.trim()),
            ("maintainer", self.maintainer.trim()),
            ("group", self.group.trim()),
            ("status", self.status.trim()),
            ("is_nonfree", self.is_nonfree.as_str()),
        ]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .collect()
    }

    /// Whether the listing has any filters.
    pub fn has_filters(&self) -> bool {
        !self.platform.trim().is_empty() || !self.facets().is_empty()
    }

    pub fn search(&self) -> (&str, bool) {
        if self.name.trim().is_empty() {
            (self.q.trim(), false)
        } else {
            (self.name.trim(), true)
        }
    }
}

/// Package search results.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PackageResults {
    pub packages: Vec<Package>,
    pub per_page: i32,
    /// Package count for browsing. Omitted for search results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,

    /// Set when `total` hit the listing count limit and is really "total or more".
    pub total_capped: bool,

    // Offset pagination (search).
    pub page: i32,
    pub has_more: bool,

    // Keyset pagination (listing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
}

/// Keyset pagination key serialized as `id:name`.
#[derive(Debug, Clone, Default)]
pub struct Cursor {
    pub id: i64,
    pub name: String,
}

impl Cursor {
    pub fn of(p: &Package) -> String {
        format!("{}:{}", p.id, p.name)
    }

    /// Parse a keyset pagination string.
    pub fn parse(s: &str) -> Self {
        match s.split_once(':') {
            Some((id, name)) => Self {
                id: id.parse().unwrap_or_default(),
                name: name.to_string(),
            },
            None => Self::default(),
        }
    }
}

/// Application configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub import: ImportConfig,

    pub app: AppConfig,
    pub db: DbConfig,

    #[serde(default)]
    pub api_results: ApiResultsConfig,

    #[serde(default)]
    pub site_results: SiteResultsConfig,
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

#[derive(Debug, Clone, Deserialize)]
pub struct SiteResultsConfig {
    #[serde(default = "default_site_per_page")]
    pub per_page: i32,
    #[serde(default = "default_site_max_per_page")]
    pub max_per_page: i32,
}

fn default_site_per_page() -> i32 {
    50
}

fn default_site_max_per_page() -> i32 {
    100
}

impl Default for SiteResultsConfig {
    fn default() -> Self {
        Self {
            per_page: default_site_per_page(),
            max_per_page: default_site_max_per_page(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImportConfig {
    #[serde(default)]
    pub families: Vec<String>,
}
