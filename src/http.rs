use std::sync::Arc;

use axum::{routing::get, Router};

use crate::handlers::{packages, Ctx};

/// Initialize HTTP routes.
pub fn init_handlers(ctx: Arc<Ctx>) -> Router {
    let pub_routes =
        Router::new().route("/repos/{repo_id}/packages", get(packages::query_packages));

    Router::new().merge(pub_routes).with_state(ctx)
}
