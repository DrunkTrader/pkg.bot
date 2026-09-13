use std::sync::Arc;

use axum::extract::State;
use axum_extra::extract::Query;
use serde::Deserialize;

use super::{json, ApiResp, Ctx, Result};

/// Maximum suggestions returned per query.
const LIMIT: usize = 15;

/// `?q=` on a suggest endpoint.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SuggestQuery {
    pub q: String,
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
