use std::{sync::Arc, time::Instant};

use axum::{
    extract::{Path, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::handlers::{packages, site, suggest, Ctx, ReqStarted};

/// Initialize HTTP routes.
pub fn init_handlers(ctx: Arc<Ctx>) -> Router {
    // JSON API.
    let mut router = Router::new()
        .route(
            "/api/repos/{repo_id}/packages",
            get(packages::query_packages),
        )
        .route("/api/suggest/licenses", get(suggest::suggest_licenses))
        .route("/api/suggest/platforms", get(suggest::suggest_platforms));

    // HMTL pages.
    if ctx.site.is_some() {
        router = router.merge(
            Router::new()
                .route("/", get(site::index))
                .route("/p/{page}", get(site::render_custom_page))
                .route("/search", get(site::render_search_form))
                .route("/repos", get(site::render_repos))
                // `/repos/{repo}.xml` is handled inside render_search as axum can't do dynamic suffixes.
                .route("/repos.xml", get(site::render_repos))
                .route("/repos/{repo}", get(site::render_search))
                .route("/repos/{repo}/{pkg}", get(site::get_package))
                .route("/repos/{repo}/{pkg}/feed.xml", get(site::get_package))
                .route("/static/{*path}", get(serve_static))
                .layer(middleware::from_fn(req_time)),
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

/// Add a timestamp to request to track elapsed time.
async fn req_time(mut req: Request, next: Next) -> Response {
    req.extensions_mut().insert(ReqStarted(Instant::now()));
    next.run(req).await
}
