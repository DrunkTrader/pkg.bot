use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use axum_extra::extract::Query;

use super::{json, paginate, total_pages, ApiErr, ApiResp, Ctx, Result};
use crate::models::{PackageQuery, PackageResults};

/// Search packages in a repository.
pub async fn query_packages(
    State(ctx): State<Arc<Ctx>>,
    Path(repo_id): Path<i64>,
    Query(mut q): Query<PackageQuery>,
) -> Result<ApiResp<PackageResults>> {
    if !ctx.repos.iter().any(|r| r.id == repo_id) {
        return Err(ApiErr::new("repo not found", StatusCode::NOT_FOUND));
    }

    if let Err(e) = q.validate() {
        return Err(ApiErr::new(e, StatusCode::BAD_REQUEST));
    }

    q.repo_id = repo_id;

    // Pagination.
    let (page, per_page, offset) = paginate(
        q.page,
        q.per_page,
        ctx.consts.api_max_per_page,
        ctx.consts.api_default_per_page,
    );

    q.page = page;
    q.offset = offset;
    q.limit = per_page;

    let (packages, total) = ctx.mgr.query_packages(&q).await?;

    Ok(json(PackageResults {
        packages,
        page,
        per_page,
        total,
        total_pages: total_pages(total, per_page),
    }))
}
