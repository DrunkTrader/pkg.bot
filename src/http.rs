use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};

use crate::handlers::{packages, site, Ctx};

/// Initialize HTTP routes.
pub fn init_handlers(ctx: Arc<Ctx>) -> Router {
    // JSON API.
    let mut router = Router::new().route(
        "/api/repos/{repo_id}/packages",
        get(packages::query_packages),
    );

    // HMTL pages.
    if ctx.site.is_some() {
        router = router.merge(
            Router::new()
                .route("/", get(site::index))
                .route("/repos/{repo}", get(site::search))
                .route("/repos/{repo}/{pkg}", get(site::package))
                .route("/static/{*path}", get(serve_static)),
        );
    } else {
        log::info!("no --site given. serving JSON APIs only");
    }

    router.with_state(ctx)
}

/// Serve static files from the site directory.
async fn serve_static(State(ctx): State<Arc<Ctx>>, Path(path): Path<String>) -> impl IntoResponse {
    let not_found = (StatusCode::NOT_FOUND, "not found").into_response();

    let Some(site) = &ctx.site else {
        return not_found;
    };


    let rel = std::path::Path::new(path.trim_start_matches('/'));
    if rel
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return not_found;
    }

    match tokio::fs::read(site.path.join("static").join(rel)).await {
        Ok(body) => (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    mime_guess::from_path(rel)
                        .first_or_octet_stream()
                        .to_string(),
                ),
                // Static URLs are cache-busted with ?v={asset_ver}.
                (header::CACHE_CONTROL, "public, max-age=604800".to_string()),
            ],
            body,
        )
            .into_response(),
        Err(_) => not_found,
    }
}
