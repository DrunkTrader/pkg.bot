use std::sync::Arc;

use axum::{
    extract::{Path, RawQuery, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::Query;

use super::{list_packages, paginate, Ctx};
use crate::models::{PackageQuery, PackageResults};

/// Landing page.
pub async fn index(State(ctx): State<Arc<Ctx>>) -> Response {
    let mut tpl_ctx = base_context(&ctx);
    tpl_ctx.insert("page_type", "index");
    tpl_ctx.insert(
        "total_packages",
        &ctx.repos.iter().map(|r| r.package_count).sum::<i64>(),
    );

    render(&ctx, "index.html", &tpl_ctx)
}

/// Standalone advanced search form.
pub async fn search_form(
    State(ctx): State<Arc<Ctx>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    Query(mut q): Query<PackageQuery>,
) -> Response {
    let mut tpl_ctx = base_context(&ctx);
    tpl_ctx.insert("page_type", "search-form");
    if let Some(slug) = params.get("repo") {
        match ctx.repo(slug) {
            Some(repo) => tpl_ctx.insert("repo", repo),
            None => return not_found(&ctx, "Unknown repository."),
        }
    }
    if let Err(e) = q.validate() {
        return render_message(&ctx, StatusCode::BAD_REQUEST, "Invalid search", e);
    }
    q.per_page = paginate(
        q.page,
        q.per_page,
        ctx.consts.site_max_per_page,
        ctx.consts.site_default_per_page,
    )
    .1;
    tpl_ctx.insert("q", &q);
    insert_search(&mut tpl_ctx, &q);
    render(&ctx, "search-form.html", &tpl_ctx)
}

/// Search results page. Takes the same query params as the JSON search API.
pub async fn search(
    State(ctx): State<Arc<Ctx>>,
    Path(repo_slug): Path<String>,
    RawQuery(raw_query): RawQuery,
    Query(mut q): Query<PackageQuery>,
) -> Response {
    let repo = match ctx.repo(&repo_slug) {
        Some(r) => r,
        None => return not_found(&ctx, "Unknown repository."),
    };

    if let Err(e) = q.validate() {
        return render_message(&ctx, StatusCode::BAD_REQUEST, "Invalid search", e);
    }

    // Pagination.
    let (page, per_page, offset) = paginate(
        q.page,
        q.per_page,
        ctx.consts.site_max_per_page,
        ctx.consts.site_default_per_page,
    );

    q.repo_id = repo.id;
    q.page = page;
    q.per_page = per_page;
    q.offset = offset;
    q.limit = per_page;

    let results = match list_packages(&ctx, repo, &q).await {
        Ok(res) => res,
        Err(e) => {
            log::error!("error querying packages: {}", e.message);
            PackageResults::default()
        }
    };

    let mut tpl_ctx = base_context(&ctx);
    tpl_ctx.insert("page_type", "search");
    tpl_ctx.insert("repo", repo);
    tpl_ctx.insert("q", &q);
    insert_search(&mut tpl_ctx, &q);
    tpl_ctx.insert("results", &results);

    // Prefix that pagination links append their own cursor or page param to,
    // retaining all the other search params.
    tpl_ctx.insert(
        "pg_url",
        &format!(
            "{}/repos/{}?{}",
            ctx.consts.root_url,
            repo.slug,
            strip_pagination(&raw_query.unwrap_or_default())
        ),
    );

    render(&ctx, "search.html", &tpl_ctx)
}

/// Individual package page.
pub async fn get_package(
    State(ctx): State<Arc<Ctx>>,
    Path((repo_slug, pkg_slug)): Path<(String, String)>,
) -> Response {
    let repo = match ctx.repo(&repo_slug) {
        Some(r) => r,
        None => return not_found(&ctx, "Unknown repository."),
    };

    let pkg = match ctx.mgr.get_package(repo.id, &pkg_slug).await {
        Ok(p) => p,
        Err(crate::manager::Error::NotFound) => {
            return not_found(&ctx, "The package does not exist in this repository.")
        }
        Err(e) => {
            log::error!("error fetching package: {}", e);
            return render_message(
                &ctx,
                StatusCode::INTERNAL_SERVER_ERROR,
                "Error",
                "Error fetching the package.",
            );
        }
    };

    let mut tpl_ctx = base_context(&ctx);
    tpl_ctx.insert("page_type", "package");
    tpl_ctx.insert("repo", repo);

    // The package's page on the repo's own website.
    if let Some(tpl) = &repo.pkg_url_template {
        tpl_ctx.insert("pkg_url", &tpl.replace("{slug}", &pkg.slug));
    }

    tpl_ctx.insert("pkg", &pkg);

    render(&ctx, "package.html", &tpl_ctx)
}

/// Template context common to all pages.
fn base_context(ctx: &Ctx) -> tera::Context {
    let mut tpl_ctx = tera::Context::new();
    tpl_ctx.insert("consts", &ctx.consts);
    tpl_ctx.insert("repos", &ctx.repos);
    tpl_ctx.insert("asset_ver", &ctx.asset_ver);

    // The search form is on every page. Give it an empty query and the
    // highest ranked repo to start with. Pages that have a query and a repo
    // of their own overwrite these.
    tpl_ctx.insert("q", &PackageQuery::default());
    insert_search(&mut tpl_ctx, &PackageQuery::default());
    if let Some(repo) = ctx.repos.first() {
        tpl_ctx.insert("repo", repo);
    }

    tpl_ctx
}

/// The search term and its scope, which the form and result links are built
/// from without having to know which of the two params was used.
fn insert_search(tpl_ctx: &mut tera::Context, q: &PackageQuery) {
    let (term, name_only) = q.search();
    tpl_ctx.insert("term", term);
    tpl_ctx.insert("scope", if name_only { "name" } else { "query" });
}

/// Render a template into an HTML response.
fn render(ctx: &Ctx, tpl: &str, tpl_ctx: &tera::Context) -> Response {
    let Some(site) = &ctx.site else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    match site.tpl.render(tpl, tpl_ctx) {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            log::error!("error rendering {}: {}", tpl, crate::init::err_chain(&e));

            (StatusCode::INTERNAL_SERVER_ERROR, "error rendering page").into_response()
        }
    }
}

/// Render the generic message page.
fn render_message(ctx: &Ctx, status: StatusCode, title: &str, message: &str) -> Response {
    let mut tpl_ctx = base_context(ctx);
    tpl_ctx.insert("page_type", "message");
    tpl_ctx.insert("title", title);
    tpl_ctx.insert("message", message);

    (status, render(ctx, "message.html", &tpl_ctx)).into_response()
}

fn not_found(ctx: &Ctx, message: &str) -> Response {
    render_message(ctx, StatusCode::NOT_FOUND, "Not found", message)
}

/// Drop pagination params from a raw query string so that pagination links can
/// append their own. Eg: "query=vim&page=3" => "query=vim&"
fn strip_pagination(raw: &str) -> String {
    raw.split('&')
        .filter(|p| {
            !p.is_empty()
                && !["page=", "after=", "before="]
                    .iter()
                    .any(|k| p.starts_with(k))
        })
        .flat_map(|p| [p, "&"])
        .collect()
}
