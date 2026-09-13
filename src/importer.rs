//! Imports a Repology PostgreSQL dump into a fresh SQLite database.

use std::{collections::HashMap, error::Error, path::Path, time::Instant};

use serde_json::json;
use sqlx::{
    sqlite::SqliteConnectOptions, Connection, Executor, PgConnection, QueryBuilder, Sqlite,
    SqliteConnection,
};
use tokio::sync::mpsc;

use crate::models::{url_template, Maintainer, IMPORT, SCHEMA};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Rows per SQLite INSERT statement.
const CHUNK: usize = 1000;

/// Repology Repo.
#[derive(sqlx::FromRow)]
struct SrcRepo {
    id: i64,
    slug: String,
    name: String,
    family: String,
    homepage_url: Option<String>,
    repolinks: Option<String>,
    pkg_url_template: Option<String>,
    source_url_template: Option<String>,
}

/// Repology package.
#[derive(sqlx::FromRow)]
struct SrcPackage {
    id: i64,
    repo: String,
    family: String,
    srcname: Option<String>,
    binnames: Option<Vec<String>>,
    trackname: String,
    visiblename: String,
    rawversion: String,
    version: String,
    maintainers: Option<Vec<String>>,
    category: Option<String>,
    comment: Option<String>,
    licenses: Option<Vec<String>>,
    effname: String,
    versionclass: i32,
    flags: i32,
    shadow: bool,
    platforms: Option<Vec<String>>,
    subrepos: Option<Vec<String>>,
    subrepo: Option<String>,
    arch: Option<String>,
    homepage_url: Option<String>,
}

/// SQLite package.
struct Package {
    id: i64,
    repo_id: i64,
    slug: String,
    name: String,
    name_norm: String,
    excerpt: Option<String>,
    pkg_base: Option<String>,
    version: String,
    version_norm: Option<String>,
    homepage_url: Option<String>,
    licenses: String,
    platforms: String,
    groups: String,
    keywords: String,
    status: &'static str,
    meta: String,
    identity_tokens: Option<String>,
    keyword_tokens: Option<String>,
    body_tokens: Option<String>,
    maintainers: Vec<i64>,
}

/// Import active Repology repos filtered by the given families.
/// In the Repology DB, it's in `repositories.metadata->family` JSONB field.
pub async fn run(dsn: &str, db_path: &Path, families: &[String]) -> Result<()> {
    if families.is_empty() {
        return Err("import.families is empty".into());
    }

    log::info!("import families: {:?}", families);

    let start = Instant::now();
    let mut db = init_db(db_path).await?;

    log::info!("connecting to PostgreSQL");
    let mut pg = PgConnection::connect(dsn).await?;

    for stmt in [
        "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
        "SET statement_timeout = 0",
        "SET idle_in_transaction_session_timeout = 0",
        "SET work_mem = '512MB'",
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

    let repos = import_repos(&mut pg, &mut db, families).await?;
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
        .create_if_missing(true);
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
    families: &[String],
) -> Result<HashMap<String, i64>> {
    let src: Vec<SrcRepo> = sqlx::query_as(&IMPORT.pg_get_repos.query)
        .bind(families)
        .fetch_all(&mut *pg)
        .await?;
    if src.is_empty() {
        return Err("no active repositories match import.families".into());
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

        let repolinks: serde_json::Value =
            serde_json::from_str(r.repolinks.as_deref().unwrap_or("[]"))
                .unwrap_or_else(|_| json!([]));

        sqlx::query(
            "INSERT INTO repos (id, slug, name, family, manager, distro, homepage_url, \
             pkg_url_template, source_url_template, meta) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(json!({ "repolinks": repolinks }).to_string())
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
    Ok(src.into_iter().map(|r| (r.slug, r.id)).collect())
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
    repos: &HashMap<String, i64>,
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
            "INSERT INTO packages (id, repo_id, slug, name, name_norm, excerpt, pkg_base, \
             version, version_norm, homepage_url, licenses, platforms, \
             \"groups\", keywords, status, meta, identity_tokens, keyword_tokens, body_tokens) ",
        );
        q.push_values(chunk, |mut b, p| {
            b.push_bind(p.id)
                .push_bind(p.repo_id)
                .push_bind(p.slug.as_str())
                .push_bind(p.name.as_str())
                .push_bind(p.name_norm.as_str())
                .push_bind(p.excerpt.as_deref())
                .push_bind(p.pkg_base.as_deref())
                .push_bind(p.version.as_str())
                .push_bind(p.version_norm.as_deref())
                .push_bind(p.homepage_url.as_deref())
                .push_bind(p.licenses.as_str())
                .push_bind(p.platforms.as_str())
                .push_bind(p.groups.as_str())
                .push_bind(p.keywords.as_str())
                .push_bind(p.status)
                .push_bind(p.meta.as_str())
                .push_bind(p.identity_tokens.as_deref())
                .push_bind(p.keyword_tokens.as_deref())
                .push_bind(p.body_tokens.as_deref());
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
    repos: &HashMap<String, i64>,
    maintainers: &HashMap<String, i64>,
) -> (Package, usize) {
    let repo_id = repos[&p.repo];
    let name = p.visiblename;
    let name_norm = normalize_name(&name);

    let licenses = uniq(p.licenses);
    let platforms = uniq(p.platforms);
    let subrepos = uniq(p.subrepos);
    let groups: Vec<String> = p.category.into_iter().collect();

    // binnames is the actual installable package name.
    let keywords: Vec<String> = uniq(p.binnames)
        .into_iter()
        .filter(|k| *k != name)
        .collect();

    let mut ids = Vec::new();
    let mut unknown = 0;
    for m in p.maintainers.iter().flatten() {
        match maintainers.get(m.as_str()) {
            Some(id) => ids.push(*id),
            None => unknown += 1,
        }
    }

    let meta = json!({
        "id": p.id,
        "family": p.family,
        "trackname": p.trackname,
        "effname": p.effname,
        "versionclass": p.versionclass,
        "flags": p.flags,
        "shadow": p.shadow,
        "subrepos": subrepos,
        // Some repos use this in their package URL templates.
        "subrepo": p.subrepo,
        "arch": p.arch,
    })
    .to_string();

    let row = Package {
        id,
        repo_id,
        // trackname is the canonical id of a package within its repo.
        // eg: python314Packages.redis vs redis in nixos.
        identity_tokens: fts_tokenize(&[&name, &name_norm, &p.trackname, &p.effname]),
        keyword_tokens: fts_tokenize(&[&keywords.join(" "), &groups.join(" ")]),
        body_tokens: fts_tokenize(&[p.comment.as_deref().unwrap_or_default()]),
        slug: p.trackname,
        name,
        name_norm,
        excerpt: p.comment,
        pkg_base: if p.family == "nix" { None } else { p.srcname },
        version: p.rawversion,
        version_norm: normalize_version(&p.version),
        homepage_url: p.homepage_url,
        licenses: json!(licenses).to_string(),
        platforms: json!(platforms).to_string(),
        groups: json!(groups).to_string(),
        keywords: json!(keywords).to_string(),
        status: if p.versionclass == 2 {
            "outdated"
        } else {
            "active"
        },
        meta,
        maintainers: ids,
    };

    (row, unknown)
}

/// Make the package page URL for nixpkgs as repology doesn't have it.
fn make_nix_pkg_url_template(slug: &str) -> Option<String> {
    let channel = match slug {
        "nix_unstable" => "unstable".to_string(),
        // nix_stable_26_05 -> 26.05
        _ => slug.strip_prefix("nix_stable_")?.replace('_', "."),
    };

    Some(format!(
        "https://search.nixos.org/packages?channel={channel}&query={{slug|quote}}#show={{slug|quote}}"
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

/// Zero-pad the first 3 numeric parts of a version into a sortable string.
fn normalize_version(v: &str) -> Option<String> {
    if v.is_empty() {
        return None;
    }

    // Remove any numeric epoch prefix ("2:1.0" -> "1.0").
    let v = v
        .split_once(':')
        .filter(|(epoch, _)| !epoch.is_empty() && epoch.chars().all(|c| c.is_ascii_digit()))
        .map_or(v, |(_, rest)| rest);

    let mut parts: Vec<String> = v
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(3)
        .map(|s| format!("{:05}", s.parse::<u64>().unwrap_or(u64::MAX).min(99999)))
        .collect();
    parts.resize(3, "00000".to_string());

    Some(parts.join("."))
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
