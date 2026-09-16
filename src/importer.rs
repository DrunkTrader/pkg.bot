//! Imports a Repology PostgreSQL dump into a fresh SQLite database.

mod licenses;

use crate::models::normalize_version;

use std::{
    collections::HashMap,
    error::Error,
    path::Path,
    time::{Duration, Instant},
};

use serde_json::json;
use sqlx::{
    sqlite::SqliteConnectOptions, ConnectOptions, Connection, Executor, PgConnection, QueryBuilder,
    Sqlite, SqliteConnection,
};
use tokio::sync::mpsc;

use crate::models::{url_template, ImportConfig, Maintainer, PackageStatus, IMPORT, SCHEMA};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Rows per SQLite INSERT statement.
const CHUNK: usize = 1000;

/// Strings to check against package/repo paths to determine `is_nonfree`.
const NONFREE_MARKERS: &[&str] = &[
    "nonfree",
    "unfree",
    "propriet",
    "commercial",
    "multiverse",
    "eula",
    "shareware",
    "freeware",
    "closedsource",
    "nonredistributable",
];

/// Repology Repo.
#[derive(sqlx::FromRow)]
struct SrcRepo {
    id: i64,
    slug: String,
    name: String,
    family: String,
    homepage_url: Option<String>,
    links: String,
    pkg_url_template: Option<String>,
    source_url_template: Option<String>,
    num_packages: i32,
    num_maintainers: i32,
    brand_color: Option<String>,
    created_at: String,
}

/// Repology package.
#[derive(sqlx::FromRow)]
struct SrcPackage {
    repo: String,
    package: String,
    subrepos: Option<Vec<String>>,
    srcname: Option<String>,
    binnames: Option<Vec<String>>,
    visiblename: String,
    rawversion: String,
    version: String,
    maintainers: Option<Vec<String>>,
    category: Option<String>,
    comment: Option<String>,
    licenses: Option<Vec<String>>,
    effname: String,
    versionclass: i32,
    platforms: Option<Vec<String>>,
    subrepo: Option<String>,
    meta: sqlx::types::Json<SrcPackageMeta>,
    homepage_url: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

/// Repology fields retained in package metadata.
#[derive(serde::Deserialize, serde::Serialize)]
struct SrcPackageMeta {
    id: i64,
}

/// SQLite package.
struct Package {
    id: i64,
    repo_id: i64,
    package: String,
    slug: String,
    name: String,
    name_norm: String,
    project_name: String,
    binary_names: String,
    excerpt: Option<String>,
    pkg_base: Option<String>,
    subrepo: Option<String>,
    version: String,
    version_norm: Option<String>,
    homepage_url: Option<String>,
    licenses: String,
    is_nonfree: Option<bool>,
    platforms: String,
    groups: String,
    keywords: String,
    status: PackageStatus,
    meta: String,
    identity_tokens: Option<String>,
    keyword_tokens: Option<String>,
    body_tokens: Option<String>,
    maintainers: Vec<i64>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

/// Import active Repology repos filtered by the given families.
/// In the Repology DB, it's in `repositories.metadata->family` JSONB field.
pub async fn run(dsn: &str, db_path: &Path, conf: &ImportConfig) -> Result<()> {
    if conf.families.is_empty() {
        return Err("import.families is empty".into());
    }

    log::info!(
        "import families: {:?} (min {} packages per repo)",
        conf.families,
        conf.min_packages
    );

    let start = Instant::now();
    let mut db = init_db(db_path).await?;

    log::info!("connecting to PostgreSQL");
    let mut pg = PgConnection::connect(dsn).await?;

    for stmt in [
        "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
        "SET statement_timeout = 0",
        "SET idle_in_transaction_session_timeout = 0",
    ] {
        pg.execute(stmt).await?;
    }

    let ok: bool = sqlx::query_scalar(&IMPORT.pg_check_libversion.query)
        .fetch_one(&mut pg)
        .await?;
    if !ok {
        return Err("the database is missing postgresql-libversion. \
                    Run: CREATE EXTENSION libversion;"
            .into());
    }

    // Drop indexes.
    let deferred = drop_triggers_idx(&mut db).await?;

    let repos = import_repos(&mut pg, &mut db, conf).await?;
    let maintainers = import_maintainers(&mut pg, &mut db).await?;
    let total = import_packages(pg, &mut db, &repos, &maintainers).await?;

    // Indexes on package_facets are built after its rows inserted.
    let (facet_indexes, indexes): (Vec<_>, Vec<_>) = deferred
        .into_iter()
        .partition(|sql| sql.contains("package_facets"));

    exec(&mut db, "rebuilt indexes", &indexes.join(";\n")).await?;
    exec(&mut db, "counted packages", &IMPORT.update_counts.query).await?;
    exec(&mut db, "built facets", &IMPORT.build_facets.query).await?;
    exec(&mut db, "indexed facets", &facet_indexes.join(";\n")).await?;
    exec(&mut db, "built search index", &IMPORT.build_fts.query).await?;
    exec(
        &mut db,
        "compacted",
        "PRAGMA optimize; PRAGMA wal_checkpoint(TRUNCATE);",
    )
    .await?;
    db.close().await?;

    log::info!(
        "imported {total} packages from {} repos in {:.1}s",
        repos.len(),
        start.elapsed().as_secs_f64()
    );

    Ok(())
}

/// Exec a SQL statement and log duraiton.
async fn exec(db: &mut SqliteConnection, label: &str, sql: &str) -> Result<()> {
    let t = Instant::now();
    db.execute(sql).await?;
    log::info!("{label} ({:.1}s)", t.elapsed().as_secs_f64());

    Ok(())
}

/// In the DB.
async fn init_db(path: &Path) -> Result<SqliteConnection> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .log_slow_statements(log::LevelFilter::Warn, Duration::from_secs(10));
    let mut db = SqliteConnection::connect_with(&opts).await?;

    db.execute(IMPORT.set_pragmas.query.as_str()).await?;
    db.execute(SCHEMA.schema.query.as_str()).await?;

    // Fail if it's not an empty DB.
    let used: Option<i64> = sqlx::query_scalar("SELECT 1 FROM repos LIMIT 1")
        .fetch_optional(&mut db)
        .await?;
    if used.is_some() {
        return Err(format!(
            "'{}' already has imported data. try on a new db created with the `install` flag.",
            path.display()
        )
        .into());
    }

    log::info!("opened {}", path.display());
    Ok(db)
}

/// Drop triggers and indexes. Rebuild them later after full row import.
async fn drop_triggers_idx(db: &mut SqliteConnection) -> Result<Vec<String>> {
    let objects: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT type, name, sql FROM sqlite_master \
         WHERE type IN ('index', 'trigger') AND sql IS NOT NULL",
    )
    .fetch_all(&mut *db)
    .await?;

    for (kind, name, _) in &objects {
        db.execute(format!("DROP {kind} \"{name}\"").as_str())
            .await?;
    }

    Ok(objects.into_iter().map(|(_, _, sql)| sql).collect())
}

/// Import repos from the Postgres db.
async fn import_repos(
    pg: &mut PgConnection,
    db: &mut SqliteConnection,
    conf: &ImportConfig,
) -> Result<HashMap<String, SrcRepo>> {
    let src: Vec<SrcRepo> = sqlx::query_as(&IMPORT.pg_get_repos.query)
        .bind(&conf.families)
        .bind(conf.min_packages)
        .fetch_all(&mut *pg)
        .await?;
    if src.is_empty() {
        return Err("no active repositories match import.families and import.min_packages".into());
    }

    let mut tx = db.begin().await?;
    let mut untemplated = Vec::new();
    for r in &src {
        // Figure out the package and source URL templates for the repo.
        let pkg_url = match r.family.as_str() {
            "nix" => make_nix_pkg_url_template(&r.slug),
            _ => r
                .pkg_url_template
                .as_deref()
                .and_then(url_template::parse_repology),
        };
        let source_url = r
            .source_url_template
            .as_deref()
            .and_then(url_template::parse_repology);
        if pkg_url.is_none() {
            untemplated.push(r.slug.as_str());
        }

        sqlx::query(
            "INSERT INTO repos (id, slug, name, family, manager, distro, homepage_url, \
             pkg_url_template, source_url_template, links, brand_color, num_packages, \
             num_maintainers, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(r.id)
        .bind(&r.slug)
        .bind(&r.name)
        .bind(&r.family)
        .bind(get_manager_name(&r.family, &r.slug))
        .bind(get_distro(&r.family, &r.slug))
        .bind(&r.homepage_url)
        .bind(&pkg_url)
        .bind(&source_url)
        .bind(&r.links)
        .bind(&r.brand_color)
        .bind(r.num_packages)
        .bind(r.num_maintainers)
        .bind(&r.created_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    if !untemplated.is_empty() {
        log::warn!(
            "{} repos have no package page URL: {}",
            untemplated.len(),
            untemplated.join(", ")
        );
    }

    log::info!("imported {} repos", src.len());
    Ok(src.into_iter().map(|r| (r.slug.clone(), r)).collect())
}

/// Import maintainers from the Postgres db.
async fn import_maintainers(
    pg: &mut PgConnection,
    db: &mut SqliteConnection,
) -> Result<HashMap<String, i64>> {
    let mut ids = HashMap::new();
    let mut slugs: HashMap<String, i64> = HashMap::new();
    let mut last = String::new();

    loop {
        let batch: Vec<String> = sqlx::query_scalar(&IMPORT.pg_get_maintainers.query)
            .bind(&last)
            .fetch_all(&mut *pg)
            .await?;
        let Some(marker) = batch.last() else { break };
        last = marker.clone();

        // Parse and normalize maintainer slug/name, eg: "Name <a@b.c>", "a@b.c" etc.
        let mut new = Vec::new();
        for raw in batch {
            let m = parse_maintainer(&raw);
            let id = match slugs.get(&m.slug) {
                Some(id) => *id,
                None => {
                    let id = slugs.len() as i64 + 1;
                    slugs.insert(m.slug.clone(), id);
                    new.push((id, m));
                    id
                }
            };
            ids.insert(raw, id);
        }

        let mut tx = db.begin().await?;
        for chunk in new.chunks(CHUNK) {
            let mut q = QueryBuilder::<Sqlite>::new(
                "INSERT INTO maintainers (id, slug, handle, name, email) ",
            );
            q.push_values(chunk, |mut b, (id, m)| {
                b.push_bind(*id)
                    .push_bind(m.slug.as_str())
                    .push_bind(m.handle.as_str())
                    .push_bind(m.name.as_deref())
                    .push_bind(m.email.as_deref());
            });
            q.build().execute(&mut *tx).await?;
        }
        tx.commit().await?;

        log::info!("imported {} maintainers", slugs.len());
    }

    Ok(ids)
}

/// Stream packages out of PostgreSQL and insert them to SQLite DB.
async fn import_packages(
    mut pg: PgConnection,
    db: &mut SqliteConnection,
    repos: &HashMap<String, SrcRepo>,
    maintainers: &HashMap<String, i64>,
) -> Result<usize> {
    let names: Vec<&String> = repos.keys().collect();
    log::info!("selecting the newest package per repo slug");
    sqlx::query(&IMPORT.pg_declare_packages.query)
        .bind(&names)
        .execute(&mut pg)
        .await?;

    // Read the next batch.
    let (tx, mut rx) = mpsc::channel::<Vec<SrcPackage>>(1);
    let reader = tokio::spawn(async move {
        loop {
            let batch: Vec<SrcPackage> = sqlx::query_as(&IMPORT.pg_fetch_packages.query)
                .fetch_all(&mut pg)
                .await?;
            if batch.is_empty() || tx.send(batch).await.is_err() {
                return Ok::<(), sqlx::Error>(());
            }
        }
    });

    let (mut total, mut unknown, mut batches) = (0, 0, 0);
    let (mut waited, mut cpu, mut io) = (0.0, 0.0, 0.0);

    loop {
        let t = Instant::now();
        let Some(batch) = rx.recv().await else { break };
        waited += t.elapsed().as_secs_f64();

        let t = Instant::now();
        let rows: Vec<Package> = batch
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                // Keep the select order so `id`s line up with the SQLite rows.
                let (row, missing) =
                    transform_package(p, (total + i) as i64 + 1, repos, maintainers);
                unknown += missing;
                row
            })
            .collect();
        cpu += t.elapsed().as_secs_f64();

        let t = Instant::now();
        insert_packages(db, &rows).await?;
        io += t.elapsed().as_secs_f64();

        total += rows.len();
        batches += 1;
        if batches % 10 == 0 {
            log::info!(
                "imported {total} packages (read {waited:.0}s, build {cpu:.0}s, write {io:.0}s)"
            );
        }
    }
    reader.await??;
    log::info!("imported {total} packages (read {waited:.0}s, build {cpu:.0}s, write {io:.0}s)");

    if unknown > 0 {
        log::warn!("skipped {unknown} package links to unknown maintainers");
    }

    Ok(total)
}

/// Insert a batch of packages into the SQLite db.
async fn insert_packages(db: &mut SqliteConnection, rows: &[Package]) -> Result<()> {
    let mut tx = db.begin().await?;

    for chunk in rows.chunks(CHUNK) {
        let mut q = QueryBuilder::<Sqlite>::new(
            "INSERT INTO packages (id, repo_id, slug, name, name_norm, excerpt, pkg_base, subrepo, \
             version, project_name, binary_names, version_norm, homepage_url, licenses, is_nonfree, platforms, \
             \"groups\", keywords, status, meta, identity_tokens, keyword_tokens, body_tokens, \
             created_at, updated_at, package) ",
        );
        q.push_values(chunk, |mut b, p| {
            b.push_bind(p.id)
                .push_bind(p.repo_id)
                .push_bind(p.slug.as_str())
                .push_bind(p.name.as_str())
                .push_bind(p.name_norm.as_str())
                .push_bind(p.excerpt.as_deref())
                .push_bind(p.pkg_base.as_deref())
                .push_bind(p.subrepo.as_deref())
                .push_bind(p.version.as_str())
                .push_bind(p.project_name.as_str())
                .push_bind(p.binary_names.as_str())
                .push_bind(p.version_norm.as_deref())
                .push_bind(p.homepage_url.as_deref())
                .push_bind(p.licenses.as_str())
                .push_bind(p.is_nonfree)
                .push_bind(p.platforms.as_str())
                .push_bind(p.groups.as_str())
                .push_bind(p.keywords.as_str())
                .push_bind(p.status)
                .push_bind(p.meta.as_str())
                .push_bind(p.identity_tokens.as_deref())
                .push_bind(p.keyword_tokens.as_deref())
                .push_bind(p.body_tokens.as_deref())
                .push_bind(p.created_at.as_deref())
                .push_bind(p.updated_at.as_deref())
                .push_bind(p.package.as_str());
        });
        q.build().execute(&mut *tx).await?;
    }

    let links: Vec<(i64, i64)> = rows
        .iter()
        .flat_map(|p| p.maintainers.iter().map(|m| (p.id, *m)))
        .collect();
    for chunk in links.chunks(CHUNK * 8) {
        let mut q = QueryBuilder::<Sqlite>::new(
            "INSERT OR IGNORE INTO package_maintainers (package_id, maintainer_id) ",
        );
        q.push_values(chunk, |mut b, (package_id, maintainer_id)| {
            b.push_bind(*package_id).push_bind(*maintainer_id);
        });
        q.build().execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Clean up and transform the Repology package struct into a `Package` row for SQLite.
fn transform_package(
    p: SrcPackage,
    id: i64,
    repos: &HashMap<String, SrcRepo>,
    maintainers: &HashMap<String, i64>,
) -> (Package, usize) {
    let repo = &repos[&p.repo];
    let name = p.visiblename;
    let name_norm = normalize_name(&name);

    let licenses = licenses::normalize(p.licenses);
    let platforms = uniq(p.platforms);
    let subrepos = uniq(p.subrepos);
    let groups: Vec<String> = p.category.into_iter().collect();

    // binnames is the actual installable package name.
    let binary_names = uniq(p.binnames);
    let keywords: Vec<&str> = binary_names
        .iter()
        .filter(|k| **k != name)
        .map(String::as_str)
        .collect();

    let mut ids = Vec::new();
    let mut unknown = 0;
    for m in p.maintainers.iter().flatten() {
        match maintainers.get(m.as_str()) {
            Some(id) => ids.push(*id),
            None => unknown += 1,
        }
    }

    let row = Package {
        id,
        repo_id: repo.id,
        // package is the canonical id of a package within its repo.
        // eg: python314Packages.redis vs redis in nixos.
        identity_tokens: fts_tokenize(&[&name, &name_norm, &p.package, &p.effname]),
        keyword_tokens: fts_tokenize(&[&keywords.join(" "), &groups.join(" ")]),
        body_tokens: fts_tokenize(&[p.comment.as_deref().unwrap_or_default()]),
        slug: make_slug(&p.package),
        name,
        name_norm,
        excerpt: p.comment,
        pkg_base: if repo.family == "nix" {
            None
        } else {
            p.srcname
        },
        subrepo: p.subrepo,
        project_name: p.effname,
        binary_names: json!(binary_names).to_string(),
        version: p.rawversion,
        version_norm: normalize_version(&p.version),
        homepage_url: p.homepage_url,
        is_nonfree: is_nonfree(&repo.family, &licenses, &subrepos),
        licenses: json!(licenses).to_string(),
        platforms: json!(platforms).to_string(),
        groups: json!(groups).to_string(),
        keywords: json!(keywords).to_string(),
        status: PackageStatus::from_versionclass(p.versionclass),
        meta: json!(p.meta.0).to_string(),
        package: p.package,
        maintainers: ids,
        created_at: p.created_at,
        updated_at: p.updated_at,
    };

    (row, unknown)
}

/// Sanitize a package name to e URL-safe slug.
/// eg: freebsd `www/nginx`, nix `emacsPackages."0blayout"` etc.
fn make_slug(package: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    // Reserve an escape char for the empty name. Escape dot-only path
    // segments so browsers don't strip them away when the slugs are in URIs.
    if package.is_empty() {
        return "~".to_string();
    }
    let dot_segment = matches!(package, "." | "..");
    let mut out = String::with_capacity(package.len());
    for b in package.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'+' | b'@')
            || (b == b'.' && !dot_segment)
        {
            out.push(b as char);
        } else {
            out.push('~');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
    out
}

/// Make the package page URL for nixpkgs as repology doesn't have it.
fn make_nix_pkg_url_template(slug: &str) -> Option<String> {
    let channel = match slug {
        "nix_unstable" => "unstable".to_string(),
        // nix_stable_26_05 -> 26.05
        _ => slug.strip_prefix("nix_stable_")?.replace('_', "."),
    };

    Some(format!(
        "https://search.nixos.org/packages?channel={channel}&query={{package|quote}}#show={{package|quote}}"
    ))
}

/// Package manager for a Repology `family`.
fn get_manager_name<'a>(family: &'a str, slug: &str) -> &'a str {
    if slug == "aur" {
        return "aur";
    }

    match family {
        "arch" => "pacman",
        "debuntu" => "apt",
        "alpine" => "apk",
        "fedora" => "dnf",
        "centos" => "yum",
        "opensuse" => "zypper",
        "gentoo" => "portage",
        "freebsd" => "pkg",
        _ => family,
    }
}

/// Distro a Repology `family`` belongs to.
fn get_distro<'a>(family: &'a str, slug: &'a str) -> &'a str {
    match family {
        "nix" => "nixos",
        // debian_12, ubuntu_24_04, ...
        "debuntu" => slug.split('_').next().unwrap_or(slug),
        _ => family,
    }
}

/// Guess if a package is non-free from its licenses and the subrepo names.
fn is_nonfree(family: &str, licenses: &[String], subrepos: &[String]) -> Option<bool> {
    let is_marked = |s: &str| {
        let s = normalize_name(s);
        NONFREE_MARKERS.iter().any(|m| s.contains(m))
    };

    let by_subrepo = subrepos.iter().any(|s| {
        s.split('/').any(|seg| {
            is_marked(seg)
                // Ubuntu special case.
                || (family == "debuntu" && normalize_name(seg) == "restricted")
        })
    });
    let by_license = licenses.iter().any(|l| is_marked(l));

    match (by_subrepo || by_license, licenses.is_empty()) {
        (true, _) => Some(true),
        (false, true) => None,
        (false, false) => Some(false),
    }
}

/// Deduplicate, sort, remove empty vals from an array of strings.
fn uniq(v: Option<Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = v.unwrap_or_default();
    out.retain(|s| !s.is_empty());
    out.sort_unstable();
    out.dedup();
    out
}

/// Normalize a string by lowercasing and making it alphanumeric-only.
fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Lowercase and split fields the way FTS5's unicode61 tokenizer does.
/// Duplicates are dropped so repeated words don't skew term frequency.
fn fts_tokenize(fields: &[&str]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let words: Vec<String> = fields
        .join(" ")
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && seen.insert(w.to_string()))
        .map(str::to_string)
        .collect();

    (!words.is_empty()).then(|| words.join(" "))
}

/// Strip surrounding quotes and unescape the string.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')));

    match inner {
        Some(v) => v
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
            .trim()
            .to_string(),
        None => s.to_string(),
    }
}

/// Split a Repology maintainer handle into its parts. It can be a plain string,
/// an e-mail id, or `Name <email>`.
fn parse_maintainer(raw: &str) -> Maintainer {
    let full = unquote(raw);

    // "Name <email>".
    let (name, address) = match full.rsplit_once('<') {
        Some((name, rest)) => match rest.trim_end().strip_suffix('>') {
            Some(addr) => (unquote(name), unquote(addr)),
            None => (String::new(), full.clone()),
        },
        None => (String::new(), full.clone()),
    };

    // Local part of an e-mail is used as a handle.
    match address.split_once('@') {
        Some((local, domain))
            if !local.is_empty() && !domain.is_empty() && !domain.contains(char::is_whitespace) =>
        {
            let handle = local.to_string();
            Maintainer {
                name: Some(if name.is_empty() {
                    handle.clone()
                } else {
                    name
                }),
                email: Some(address.clone()),
                slug: address,
                handle,
                url: None,
            }
        }
        _ => Maintainer {
            handle: full.clone(),
            name: Some(full.clone()),
            slug: full,
            email: None,
            url: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintainers() {
        let m = parse_maintainer("John Doe <jd@x.com>");
        assert_eq!((m.slug.as_str(), m.handle.as_str()), ("jd@x.com", "jd"));
        assert_eq!(m.name.as_deref(), Some("John Doe"));
        assert_eq!(m.email.as_deref(), Some("jd@x.com"));

        let m = parse_maintainer("\"Doe, John\" <jd@x.com>");
        assert_eq!(m.name.as_deref(), Some("Doe, John"));

        let m = parse_maintainer("jd@x.com");
        assert_eq!((m.slug.as_str(), m.handle.as_str()), ("jd@x.com", "jd"));
        assert_eq!(m.name.as_deref(), Some("jd"));

        let m = parse_maintainer("  someone  ");
        assert_eq!((m.slug.as_str(), m.handle.as_str()), ("someone", "someone"));
        assert_eq!(m.name.as_deref(), Some("someone"));
        assert_eq!(m.email, None);

        assert_eq!(parse_maintainer("@user").slug, "@user");
        assert_eq!(parse_maintainer("a@b c").slug, "a@b c");
    }

    #[test]
    fn slugs() {
        for name in [
            "gtk+",
            "python314Packages.redis",
            "node@22",
            "foo-bar_1",
            ".foo-",
        ] {
            assert_eq!(make_slug(name), name);
        }
        for (name, expected) in [
            ("www/nginx", "www~2Fnginx"),
            ("a//b", "a~2F~2Fb"),
            ("a:b", "a~3Ab"),
            ("a b", "a~20b"),
            ("emacsPackages.\"0blayout\"", "emacsPackages.~220blayout~22"),
            ("/foo/", "~2Ffoo~2F"),
            (".", "~2E"),
            ("..", "~2E~2E"),
            ("", "~"),
            ("~", "~7E"),
            ("~2F", "~7E2F"),
            ("%2F", "~252F"),
            ("é", "~C3~A9"),
        ] {
            assert_eq!(make_slug(name), expected);
        }
    }

    #[test]
    fn slug_collisions() {
        let names = [
            "a-b", "a/b", "a//b", "a:b", "a b", "a\\b", "a~2Fb", "foo", "\"foo\"", "/foo/",
            "-foo-", ".foo.", "", ".", "..", "猫", "犬", "~", "~2E", "~2E~2E", "%2F",
        ];
        let slugs: std::collections::HashSet<_> = names.iter().map(|s| make_slug(s)).collect();
        assert_eq!(slugs.len(), names.len());
    }

    #[test]
    fn nonfree() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        assert_eq!(is_nonfree("arch", &[], &[]), None);
        assert_eq!(is_nonfree("arch", &l(&["MIT"]), &l(&["main"])), Some(false));
        assert_eq!(is_nonfree("nix", &l(&["Unfree"]), &[]), Some(true));
        assert_eq!(is_nonfree("gentoo", &l(&["MSttfEULA"]), &[]), Some(true));
        assert_eq!(
            is_nonfree("arch", &l(&["custom:proprietary"]), &[]),
            Some(true)
        );
        assert_eq!(
            is_nonfree("debuntu", &[], &l(&["noble/multiverse"])),
            Some(true)
        );
        assert_eq!(
            is_nonfree("debuntu", &l(&["GPL-2"]), &l(&["sid/non-free"])),
            Some(true)
        );
        assert_eq!(
            is_nonfree("fedora", &l(&["GPL-2"]), &l(&["nonfree/tainted"])),
            Some(true)
        );
        assert_eq!(
            is_nonfree("debuntu", &[], &l(&["trixie/non-free-firmware"])),
            Some(true)
        );
        assert_eq!(is_nonfree("arch", &l(&["Non_Free"]), &[]), Some(true));
        assert_eq!(
            is_nonfree("debuntu", &l(&["GPL-2"]), &l(&["noble/restricted"])),
            Some(true)
        );
        assert_eq!(
            is_nonfree("openmandriva", &l(&["GPL-2"]), &l(&["restricted/release"])),
            Some(false)
        );
        assert_eq!(
            is_nonfree("debuntu", &l(&["GPL-2"]), &l(&["sid/contrib"])),
            Some(false)
        );
        assert_eq!(
            is_nonfree("arch", &l(&["Unrestricted Use"]), &[]),
            Some(false)
        );
    }

    #[test]
    fn versions() {
        assert_eq!(normalize_version(""), None);
        assert_eq!(
            normalize_version("2.1.1").as_deref(),
            Some("00002.00001.00001")
        );
        assert_eq!(
            normalize_version("2:1.0").as_deref(),
            Some("00001.00000.00000")
        );
        assert_eq!(
            normalize_version("1.2.3.4").as_deref(),
            Some("00001.00002.00003")
        );
        assert_eq!(
            normalize_version("unstable").as_deref(),
            Some("00000.00000.00000")
        );
        assert_eq!(
            normalize_version("999999.1").as_deref(),
            Some("99999.00001.00000")
        );
    }

    #[test]
    fn names_and_tokens() {
        assert_eq!(normalize_name("Foo-Bar_1"), "foobar1");
        assert_eq!(
            fts_tokenize(&["foo foo bar", "bar_baz"]).as_deref(),
            Some("foo bar baz")
        );
        assert_eq!(fts_tokenize(&["", "  "]), None);
    }
}
