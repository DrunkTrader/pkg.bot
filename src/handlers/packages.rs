use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use axum_extra::extract::Query;

use super::{json, list_packages, paginate, ApiErr, ApiResp, Ctx, Result};
use crate::models::{PackageQuery, PackageResults};

/// Search packages in a repository.
pub async fn query_packages(
    State(ctx): State<Arc<Ctx>>,
    Path(repo_id): Path<i64>,
    Query(mut q): Query<PackageQuery>,
) -> Result<ApiResp<PackageResults>> {
    let Some(repo) = ctx.repos.iter().find(|r| r.id == repo_id) else {
        return Err(ApiErr::new("repo not found", StatusCode::NOT_FOUND));
    };

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

    Ok(json(list_packages(&ctx, repo, &q).await?))
}
