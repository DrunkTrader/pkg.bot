use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use axum_extra::extract::Query;
use serde::Deserialize;

use super::{json, paginate, ApiErr, ApiResp, Ctx, Result};
use crate::{
    manager::Error,
    models::{Cursor, Package, PackageQuery, PackageResults, Repo, RepoQuery, Sort},
};

/// Maximum suggestions returned per query.
const LIMIT: usize = 15;

/// `?q=` on a suggest endpoint.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SuggestQuery {
    pub q: String,
}

/// Get the list of all repositories.
pub async fn get_repos(
    State(ctx): State<Arc<Ctx>>,
    Query(filter): Query<RepoQuery>,
) -> Result<ApiResp<Vec<Repo>>> {
    let repos = ctx.mgr.get_repos(&Sort::asc("name"), &filter).await?;
    Ok(json(repos))
}

/// Get a repository by its slug.
pub async fn get_repo(
    State(ctx): State<Arc<Ctx>>,
    Path(repo_slug): Path<String>,
) -> Result<ApiResp<Repo>> {
    Ok(json(get_repo_by_slug(&ctx, &repo_slug).await?))
}

/// Search packages in a repository.
pub async fn query_packages(
    State(ctx): State<Arc<Ctx>>,
    Path(repo_slug): Path<String>,
    Query(mut q): Query<PackageQuery>,
) -> Result<ApiResp<PackageResults>> {
    let (_, results) = search_packages(
        &ctx,
        &repo_slug,
        &mut q,
        ctx.consts.api_max_per_page,
        ctx.consts.api_default_per_page,
    )
    .await?;
    Ok(json(results))
}

/// Get a package.
pub async fn get_package(
    State(ctx): State<Arc<Ctx>>,
    Path((repo_slug, pkg_slug)): Path<(String, String)>,
) -> Result<ApiResp<Package>> {
    let (_, package) = get_package_by_slug(&ctx, &repo_slug, &pkg_slug).await?;
    Ok(json(package))
}

/// Suggest license values.
pub async fn suggest_licenses(
    State(ctx): State<Arc<Ctx>>,
    Query(q): Query<SuggestQuery>,
) -> Result<ApiResp<Vec<String>>> {
    Ok(json(ctx.licenses.query(&q.q, LIMIT)))
}

/// Suggest platform values.
pub async fn suggest_platforms(
    State(ctx): State<Arc<Ctx>>,
    Query(q): Query<SuggestQuery>,
) -> Result<ApiResp<Vec<String>>> {
    Ok(json(ctx.platforms.query(&q.q, LIMIT)))
}

/// Fetch a page of packages.
pub async fn list_packages(ctx: &Ctx, repo: &Repo, q: &PackageQuery) -> Result<PackageResults> {
    if !q.search().0.is_empty() {
        let (packages, has_more) = ctx.mgr.search_packages(q).await?;

        return Ok(PackageResults {
            packages,
            per_page: q.limit,
            total: None,
            total_capped: false,
            page: q.page,
            has_more,
            next: None,
            prev: None,
        });
    }

    let (packages, has_more) = ctx.mgr.get_packages(q).await?;
    let (total, total_capped) = if q.has_filters() {
        ctx.mgr.count_packages(q).await?
    } else {
        (repo.package_count, false)
    };

    // Keyset pagination.
    let (first, last) = (
        packages.first().map(Cursor::of),
        packages.last().map(Cursor::of),
    );
    let (next, prev) = if q.before.is_empty() {
        (
            if has_more { last } else { None },
            if q.after.is_empty() { None } else { first },
        )
    } else {
        (last, if has_more { first } else { None })
    };

    Ok(PackageResults {
        packages,
        per_page: q.limit,
        total: Some(total),
        total_capped,
        page: 0,
        has_more,
        next,
        prev,
    })
}

/// Get a repo by its slug.
async fn get_repo_by_slug(ctx: &Ctx, slug: &str) -> Result<Repo> {
    match ctx.mgr.get_repo(None, Some(slug)).await {
        Ok(repo) => Ok(repo),
        Err(Error::NotFound) => Err(ApiErr::new("Unknown repository.", StatusCode::NOT_FOUND)),
        Err(e) => Err(e.into()),
    }
}

/// Get a package by its repo + its own slug.
pub async fn get_package_by_slug(
    ctx: &Ctx,
    repo_slug: &str,
    pkg_slug: &str,
) -> Result<(Repo, Package)> {
    let repo = get_repo_by_slug(ctx, repo_slug).await?;
    let package = match ctx.mgr.get_package(repo.id, pkg_slug).await {
        Ok(package) => package,
        Err(Error::NotFound) => {
            return Err(ApiErr::new(
                "The package does not exist in this repository.",
                StatusCode::NOT_FOUND,
            ))
        }
        Err(e) => return Err(e.into()),
    };

    Ok((repo, package))
}

/// Search packages with filters.
pub async fn search_packages(
    ctx: &Ctx,
    slug: &str,
    q: &mut PackageQuery,
    max_per_page: i32,
    default_per_page: i32,
) -> Result<(Repo, PackageResults)> {
    let repo = get_repo_by_slug(ctx, slug).await?;

    q.validate()
        .map_err(|e| ApiErr::new(e, StatusCode::BAD_REQUEST))?;
    let (page, per_page, offset) = paginate(q.page, q.per_page, max_per_page, default_per_page);
    q.repo_id = repo.id;
    q.page = page;
    q.per_page = per_page;
    q.offset = offset;
    q.limit = per_page;

    let results = list_packages(ctx, &repo, q).await?;
    Ok((repo, results))
}
