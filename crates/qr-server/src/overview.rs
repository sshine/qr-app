//! Every label that has been made, at one unlikely address.
//!
//! The landing page deliberately does not list codes: the capability scheme exists so a
//! visitor who does not hold a sticker cannot enumerate the boxes. This page is the
//! deliberate exception — it is for the person who prints the sheets, and it is reached
//! by a URL that is not linked from anywhere. It exposes ids, titles and thumbnails, never
//! a secret, so it cannot be used to open anything.
//!
//! A card is one physical sticker. A code that has been released and reclaimed, or hidden
//! and shown again, still has one card; its history is summarised on it rather than
//! repeated as another row.

use std::collections::{BTreeSet, HashMap};

use axum::{
    Router,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use maud::{Markup, html};
use qr_core::{BlobKey, CodeId, Location, Tag, model::EventKind, store};
use qr_handler::{
    AppState,
    shell::{self, Page},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

/// The one page not reached by scanning, and the one place every used code is listed.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/kiksmiks", get(index))
        .route("/kiksmiks/{code}", get(label_page))
        .route("/kiksmiks/{code}/delete", post(delete_label))
        .route("/kiksmiks/{code}/restore", post(restore_label))
        .route("/kiksmiks/{code}/title", post(set_title))
        .route("/kiksmiks/{code}/tags", post(add_tag))
        .route("/kiksmiks/{code}/tags/{tag}/remove", post(remove_tag))
        .route("/kiksmiks/{code}/place", post(set_place))
        .route("/kiksmiks/{code}/place/clear", post(clear_place))
        .route("/kiksmiks/{code}/release", post(release_label))
        .route("/kiksmiks/{code}/recover", post(recover_label))
        .route("/kiksmiks/photo/{photo}/thumb", get(thumb))
        .with_state(state)
}

/// What the query string asks to see.
#[derive(Debug, Default)]
struct Filters {
    text: String,
    show_deleted: bool,
    tags: BTreeSet<Uuid>,
    places: BTreeSet<Uuid>,
}

impl Filters {
    /// Keys are `tag_<uuid>` and `loc_<uuid>` rather than repeated `tag` fields, because
    /// a plain form with checkboxes submits repeated names and axum's query extractor
    /// wants a map.
    fn parse(raw: Option<&str>) -> Self {
        let mut filters = Self::default();
        for (key, value) in url::form_urlencoded::parse(raw.unwrap_or("").as_bytes()) {
            match key.as_ref() {
                "q" => filters.text = value.into_owned(),
                "deleted" => filters.show_deleted = value == "1",
                _ => {
                    if let Some(id) = key
                        .strip_prefix("tag_")
                        .and_then(|id| Uuid::parse_str(id).ok())
                    {
                        filters.tags.insert(id);
                    } else if let Some(id) = key
                        .strip_prefix("loc_")
                        .and_then(|id| Uuid::parse_str(id).ok())
                    {
                        filters.places.insert(id);
                    }
                }
            }
        }
        filters
    }

    fn admits(&self, label: &Label, needle: &str) -> bool {
        if !self.show_deleted && label.deleted_at.is_some() {
            return false;
        }
        if !self.tags.is_empty() && !label.tags.iter().any(|tag| self.tags.contains(&tag.id)) {
            return false;
        }
        if !self.places.is_empty()
            && !label
                .place
                .as_ref()
                .is_some_and(|place| self.places.contains(&place.id))
        {
            return false;
        }
        if !needle.is_empty() && !haystack(label).contains(needle) {
            return false;
        }
        true
    }
}

/// Everything a card can search, lowercased once.
fn haystack(label: &Label) -> String {
    let mut parts = vec![label.code_id.to_string().to_lowercase()];
    if let Some(title) = label.binding.as_ref().and_then(|used| used.title.as_ref()) {
        parts.push(title.to_lowercase());
    }
    if let Some(place) = &label.place {
        parts.push(place.name.to_lowercase());
        if let Some(address) = &place.address {
            parts.push(address.to_lowercase());
        }
    }
    parts.extend(label.tags.iter().map(|tag| tag.name.to_lowercase()));
    if let Some(forward) = &label.forward {
        parts.push(forward.to_lowercase());
    }
    if let Some(link) = &label.link {
        parts.push(link.to_string().to_lowercase());
    }
    parts.join(" ")
}

#[derive(Debug)]
struct Place {
    id: Uuid,
    name: String,
    address: Option<String>,
    lat: f64,
    lon: f64,
}

#[derive(Debug)]
struct Use {
    id: Uuid,
    handler_id: String,
    title: Option<String>,
    released_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct Photos {
    top: Uuid,
    taken_at: Option<DateTime<Utc>>,
    total: i64,
}

#[derive(Debug)]
struct Label {
    code_id: CodeId,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    /// How many times this sticker has been hidden, not how many times it has been
    /// scanned: the card is grouped by code, and this is the history that would otherwise
    /// make several identical rows.
    deletions: i64,
    place: Option<Place>,
    binding: Option<Use>,
    tags: Vec<Tag>,
    photos: Option<Photos>,
    forward: Option<String>,
    link: Option<CodeId>,
}

impl Label {
    fn is_released(&self) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|used| used.released_at.is_some())
    }
}

/// Loads the labels the overview shows, or exactly one label for its page.
///
/// The gallery is a list of labels that have been used at least once. A code that has
/// only been printed is not a label yet: it has no title, tags or place, and listing
/// every blank sticker is what made the old page mostly "Unclaimed label" rows. The
/// detail page uses the same query with an id, so it sees the same joins and cannot show
/// something the gallery would not.
async fn load(state: &AppState, only: Option<&CodeId>) -> qr_core::Result<Vec<Label>> {
    let code = only.map(CodeId::as_str);
    let rows = sqlx::query!(
        r#"
        SELECT c.id AS code_id, c.created_at, c.revoked_at, c.deleted_at,
               l.id AS "location_id?", l.name AS "location_name?",
               l.address AS "location_address?", l.lat AS "location_lat?",
               l.lon AS "location_lon?",
               b.id AS "binding_id?", b.handler_id AS "handler_id?", b.title AS "title?",
               b.released_at AS "released_at?",
               (SELECT count(*) FROM events e
                WHERE e.code_id = c.id AND e.kind = 'code.deleted') AS "deletions!"
        FROM codes c
        LEFT JOIN locations l ON l.id = c.location_id
        LEFT JOIN LATERAL (
            SELECT id, handler_id, title, created_at, released_at
            FROM bindings
            WHERE code_id = c.id
            ORDER BY (released_at IS NULL) DESC, created_at DESC
            LIMIT 1
        ) b ON true
        WHERE EXISTS (SELECT 1 FROM bindings used_b WHERE used_b.code_id = c.id)
          AND ($1::text IS NULL OR c.id = $1)
        ORDER BY c.created_at DESC, c.id DESC
        "#,
        code,
    )
    .fetch_all(state.store().pool())
    .await?;

    let tag_rows = sqlx::query!(
        r#"
        SELECT ct.code_id, t.id, t.name, t.created_at
        FROM code_tags ct
        JOIN tags t ON t.id = ct.tag_id
        ORDER BY lower(t.name), t.created_at
        "#,
    )
    .fetch_all(state.store().pool())
    .await?;

    let binding_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.binding_id).collect();

    let photo_rows = if binding_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query!(
            r#"
            SELECT DISTINCT ON (binding_id) binding_id, id, taken_at,
                   count(*) OVER (PARTITION BY binding_id) AS "total!"
            FROM photo_items
            WHERE deleted_at IS NULL AND binding_id = ANY($1)
            ORDER BY binding_id, id DESC
            "#,
            &binding_ids,
        )
        .fetch_all(state.store().pool())
        .await?
    };

    let forward_rows = if binding_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query!(
            r#"
            SELECT binding_id, url
            FROM forward_targets
            WHERE superseded_at IS NULL AND binding_id = ANY($1)
            "#,
            &binding_ids,
        )
        .fetch_all(state.store().pool())
        .await?
    };

    let link_rows = if binding_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query!(
            r#"
            SELECT binding_id, target_code_id
            FROM code_links
            WHERE binding_id = ANY($1)
            "#,
            &binding_ids,
        )
        .fetch_all(state.store().pool())
        .await?
    };

    let mut tags: HashMap<String, Vec<Tag>> = HashMap::new();
    for row in tag_rows {
        tags.entry(row.code_id).or_default().push(Tag {
            id: row.id,
            name: row.name,
            created_at: row.created_at,
        });
    }

    let mut photos: HashMap<Uuid, Photos> = HashMap::new();
    for row in photo_rows {
        photos.insert(
            row.binding_id,
            Photos {
                top: row.id,
                taken_at: row.taken_at,
                total: row.total,
            },
        );
    }

    let mut forwards: HashMap<Uuid, String> = HashMap::new();
    for row in forward_rows {
        forwards.insert(row.binding_id, row.url);
    }

    let mut links: HashMap<Uuid, CodeId> = HashMap::new();
    for row in link_rows {
        if let Ok(target) = CodeId::parse(&row.target_code_id) {
            links.insert(row.binding_id, target);
        }
    }

    let labels: Vec<Label> = rows
        .into_iter()
        .map(|row| {
            let code_id = CodeId::parse(&row.code_id)?;
            let binding = row.binding_id.map(|id| Use {
                id,
                handler_id: row.handler_id.unwrap_or_default(),
                title: row.title,
                released_at: row.released_at,
            });
            Ok(Label {
                created_at: row.created_at,
                revoked_at: row.revoked_at,
                deleted_at: row.deleted_at,
                deletions: row.deletions,
                place: row.location_id.map(|id| Place {
                    id,
                    name: row.location_name.unwrap_or_default(),
                    address: row.location_address,
                    lat: row.location_lat.unwrap_or(0.0),
                    lon: row.location_lon.unwrap_or(0.0),
                }),
                photos: row.binding_id.and_then(|id| photos.remove(&id)),
                forward: row.binding_id.and_then(|id| forwards.remove(&id)),
                link: row.binding_id.and_then(|id| links.remove(&id)),
                tags: tags.remove(&row.code_id).unwrap_or_default(),
                binding,
                code_id,
            })
        })
        .collect::<qr_core::Result<_>>()?;

    // The migration removes every pre-checksum row. Filtering here as well means a
    // database that has not been migrated yet shows a smaller gallery rather than a
    // page that fails on a code the public routes would refuse.
    Ok(labels
        .into_iter()
        .filter(|label| label.code_id.checksum_ok())
        .collect())
}

/// `GET /kiksmiks` — the gallery, filtered by the query string.
async fn index(State(state): State<AppState>, RawQuery(raw): RawQuery) -> Response {
    let filters = Filters::parse(raw.as_deref());
    let needle = filters.text.trim().to_lowercase();

    let labels = match load(&state, None).await {
        Ok(labels) => labels,
        Err(error) => {
            return (StatusCode::SERVICE_UNAVAILABLE, format!("{error}\n")).into_response();
        }
    };
    let labels: Vec<Label> = labels
        .into_iter()
        .filter(|label| filters.admits(label, &needle))
        .collect();

    let tags = store::tags(state.store().pool()).await.unwrap_or_default();
    let places = store::locations(state.store().pool())
        .await
        .unwrap_or_default();
    let back = raw.unwrap_or_default();

    shell::render(
        Page::new(
            "All labels",
            html! {
                main class="overview" {
                    header class="overview-head" {
                        h1 { "All labels" }
                        p class="hint" {
                            (labels.len())
                            @if labels.len() == 1 { " label" } @else { " labels" }
                            " · "
                            a href="/kiksmiks" { "Everything" }
                            " · "
                            a href="/sheet" { "Print a sheet" }
                        }
                    }

                    (filter_form(&filters, &tags, &places))

                    @if labels.is_empty() {
                        p class="empty" { "Nothing matches those filters." }
                    } @else {
                        ul class="label-grid" {
                            @for label in &labels {
                                (card(&state, label, &back))
                            }
                        }
                    }
                }
            },
        )
        .with_state(json!({ "overview": true })),
    )
    .into_response()
}

fn filter_form(filters: &Filters, tags: &[Tag], places: &[Location]) -> Markup {
    html! {
        form class="filters" method="get" action="/kiksmiks" {
            div class="filter-search" {
                label {
                    "Search"
                    input type="text" name="q" value=(filters.text)
                        autocomplete="off" spellcheck="false"
                        placeholder="code, title, tag, place or destination";
                }
                button type="submit" { "Filter" }
                a class="clear" href="/kiksmiks" { "Clear" }
            }

            @if !tags.is_empty() {
                fieldset class="filter-group" {
                    legend { "Tags" }
                    div class="filter-chips" {
                        @for tag in tags {
                            label class="filter-chip" {
                                input type="checkbox" name={ "tag_" (tag.id) } value="1"
                                    checked[filters.tags.contains(&tag.id)];
                                span { (tag.name) }
                            }
                        }
                    }
                }
            }

            @if !places.is_empty() {
                fieldset class="filter-group" {
                    legend { "Places" }
                    div class="filter-chips" {
                        @for place in places {
                            label class="filter-chip" {
                                input type="checkbox" name={ "loc_" (place.id) } value="1"
                                    checked[filters.places.contains(&place.id)];
                                span {
                                    (place.name)
                                    @if let Some(address) = &place.address {
                                        " — " (address)
                                    }
                                }
                            }
                        }
                    }
                }
            }

            label class="show-deleted" {
                input type="checkbox" name="deleted" value="1" checked[filters.show_deleted];
                "Show deleted labels"
            }
        }
    }
}

fn card(state: &AppState, label: &Label, back: &str) -> Markup {
    let code = &label.code_id;
    let deleted = label.deleted_at.is_some();
    let handler = label
        .binding
        .as_ref()
        .and_then(|used| state.registry().by_name(&used.handler_id))
        .map(|handler| handler.name().to_owned());
    let title = label
        .binding
        .as_ref()
        .and_then(|used| used.title.clone())
        .or_else(|| handler.clone())
        .unwrap_or_else(|| "Unused label".to_owned());
    let class = if deleted {
        "label-card deleted"
    } else {
        "label-card"
    };
    let href = detail_href(code, back);

    html! {
        li class=(class) {
            (thumbnail(label, handler.as_deref(), &href))

            div class="card-text" {
                div class="card-top" {
                    h2 { a href=(href) { (title) } }
                }
                p class="code-id" { (code.grouped()) }

                p class="meta" {
                    @if let Some(place) = &label.place {
                        span class="place" { (place.name) }
                        " · "
                    }
                    (label.created_at.format("%-d %b %Y"))
                    @if label.is_released() {
                        " · released"
                    }
                    @if label.revoked_at.is_some() {
                        " · revoked"
                    }
                    @if deleted {
                        " · "
                        strong {
                            @if label.deletions > 1 {
                                "deleted " (label.deletions) " times"
                            } @else {
                                "deleted"
                            }
                        }
                    }
                }

                @if let Some(place) = &label.place {
                    p class="coords" {
                        (format!("{:.4}, {:.4}", place.lat, place.lon))
                    }
                }

                @if let Some(photos) = &label.photos {
                    p class="detail" {
                        @if photos.total == 1 {
                            "1 photo"
                        } @else {
                            (photos.total) " photos"
                        }
                        @if let Some(taken) = photos.taken_at {
                            " · " (taken.format("%-d %b %Y"))
                        }
                    }
                } @else if let Some(url) = &label.forward {
                    p class="detail" { "→ " (url) }
                } @else if let Some(target) = &label.link {
                    p class="detail" { "Links to " (target.grouped()) }
                } @else if label
                    .binding
                    .as_ref()
                    .is_some_and(|used| used.handler_id == "photo-album")
                {
                    p class="detail" { "No photos yet" }
                } @else if let Some(use_) = &label.binding {
                    p class="detail" { (use_.handler_id) }
                }

                @if !label.tags.is_empty() {
                    ul class="card-tags" {
                        @for tag in &label.tags {
                            li {
                                a href={ "/kiksmiks?tag_" (tag.id) "=1" } { (tag.name) }
                            }
                        }
                    }
                }

                div class="card-actions" {
                    @if deleted {
                        form method="post" action={ "/kiksmiks/" (code) "/restore" } {
                            input type="hidden" name="back" value=(back);
                            button type="submit" { "Restore" }
                        }
                    } @else {
                        form method="post" action={ "/kiksmiks/" (code) "/delete" } {
                            input type="hidden" name="back" value=(back);
                            button type="submit" { "Delete" }
                        }
                    }
                }
            }
        }
    }
}

fn thumbnail(label: &Label, handler: Option<&str>, href: &str) -> Markup {
    let opens = format!("Open label {}", label.code_id.grouped());
    if let Some(photos) = &label.photos {
        html! {
            a class="thumb photo" href=(href) aria-label=(opens) {
                img src={ "/kiksmiks/photo/" (photos.top) "/thumb" } alt=""
                    loading="lazy" decoding="async";
                @if photos.total > 1 {
                    span class="photo-count" { "+" (photos.total - 1) }
                }
            }
        }
    } else if label.forward.is_some() {
        html! {
            a class="thumb placeholder" href=(href) aria-label=(opens) {
                span class="glyph" aria-hidden="true" { "→" }
            }
        }
    } else if label.link.is_some() {
        html! {
            a class="thumb placeholder" href=(href) aria-label=(opens) {
                span class="glyph" aria-hidden="true" { "∞" }
            }
        }
    } else {
        html! {
            a class="thumb placeholder" href=(href) aria-label=(opens) {
                span class="glyph" aria-hidden="true" { "▤" }
                span class="placeholder-word" { (handler.unwrap_or("Unused")) }
            }
        }
    }
}

fn detail_href(code: &CodeId, back: &str) -> String {
    if back.is_empty() {
        return format!("/kiksmiks/{code}");
    }
    let encoded: String = url::form_urlencoded::byte_serialize(back.as_bytes()).collect();
    format!("/kiksmiks/{code}?back={encoded}")
}

#[derive(Deserialize)]
struct Back {
    #[serde(default)]
    back: String,
}

fn back_to(back: &str) -> String {
    if back.is_empty() {
        "/kiksmiks".to_owned()
    } else if back.starts_with("/kiksmiks") && !back.starts_with("//") {
        // A detail page's own path, which carries the gallery's filters in its query.
        back.to_owned()
    } else {
        format!("/kiksmiks?{back}")
    }
}

/// `GET /kiksmiks/{code}` — one label, with the controls the gallery does not have room
/// for. This is not a capability page: it shows metadata and edits the overview's own
/// record, never a secret and never the sticker's content.
async fn label_page(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Ok(code) = CodeId::parse_checksummed(&raw) else {
        return not_found();
    };

    let labels = match load(&state, Some(&code)).await {
        Ok(labels) => labels,
        Err(error) => return store_response(&error),
    };
    let Some(label) = labels.into_iter().next() else {
        return not_found();
    };

    let tags = store::tags(state.store().pool()).await.unwrap_or_default();
    let places = store::locations(state.store().pool())
        .await
        .unwrap_or_default();
    let history = store::released_bindings(state.store().pool(), &code)
        .await
        .unwrap_or_default();
    let photos = match &label.binding {
        Some(used) => qr_photos::store::list(state.store().pool(), used.id, None, 60)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let back = raw_query.unwrap_or_default();
    let encoded_back: String = url::form_urlencoded::byte_serialize(back.as_bytes()).collect();
    let all_labels = back_to(&back);
    let self_path = if back.is_empty() {
        format!("/kiksmiks/{code}")
    } else {
        format!("/kiksmiks/{code}?back={encoded_back}")
    };

    let handler = label
        .binding
        .as_ref()
        .and_then(|used| state.registry().by_name(&used.handler_id))
        .map(|handler| handler.name().to_owned());
    let title = label
        .binding
        .as_ref()
        .and_then(|used| used.title.clone())
        .or_else(|| handler.clone())
        .unwrap_or_else(|| "Unused label".to_owned());
    let deleted = label.deleted_at.is_some();
    let active = label
        .binding
        .as_ref()
        .filter(|used| used.released_at.is_none());
    let held: Vec<Uuid> = label.tags.iter().map(|tag| tag.id).collect();
    let available: Vec<&Tag> = tags.iter().filter(|tag| !held.contains(&tag.id)).collect();

    shell::render(
        Page::new(
            format!("{title} · Labels"),
            html! {
                main class="label-page" {
                    nav class="label-nav" {
                        a href=(all_labels) { "← All labels" }
                    }

                    header class="label-head" {
                        h1 { (title) }
                        p class="code-id" { (code.grouped()) }
                        p class="meta" {
                            @if let Some(place) = &label.place {
                                span class="place" { (place.name) }
                                " · "
                            }
                            (label.created_at.format("%-d %b %Y"))
                            @if label.is_released() { " · released" }
                            @if label.revoked_at.is_some() { " · revoked" }
                            @if deleted {
                                " · "
                                strong {
                                    @if label.deletions > 1 {
                                        "deleted " (label.deletions) " times"
                                    } @else {
                                        "deleted"
                                    }
                                }
                            }
                        }
                    }

                    div class="label-actions" {
                        @if active.is_some() {
                            a class="button" href={ "/c/" (code) } { "Open this label" }
                        }
                        @if deleted {
                            form method="post" action={ "/kiksmiks/" (code) "/restore" } {
                                input type="hidden" name="back" value=(self_path);
                                button type="submit" { "Restore" }
                            }
                        } @else {
                            form method="post" action={ "/kiksmiks/" (code) "/delete" } {
                                input type="hidden" name="back" value=(self_path);
                                button type="submit" { "Delete" }
                            }
                        }
                        @if active.is_some() {
                            form method="post" action={ "/kiksmiks/" (code) "/release" } {
                                input type="hidden" name="back" value=(self_path);
                                button type="submit" { "Release" }
                            }
                        }
                    }

                    @if active.is_some() {
                        p class="hint" {
                            "Opening the label's page needs this browser to have scanned the
                             sticker. The overview never holds its secret."
                        }
                    }

                    @if label.forward.is_some() || label.link.is_some() {
                        p class="detail" {
                            @if let Some(url) = &label.forward {
                                "Destination: " (url)
                            } @else if let Some(target) = &label.link {
                                "Links to " (target.grouped())
                            }
                        }
                    }

                    @if !photos.is_empty() {
                        section class="label-section" {
                            h2 { "Photos" }
                            ul class="thumb-grid" {
                                @for photo in &photos {
                                    li {
                                        img src={ "/kiksmiks/photo/" (photo.id) "/thumb" } alt=""
                                            loading="lazy" decoding="async";
                                    }
                                }
                            }
                        }
                    }

                    @if let Some(used) = &label.binding {
                        section class="label-section" {
                            h2 { "Title" }
                            form class="edit-form" method="post"
                                action={ "/kiksmiks/" (code) "/title" } {
                                input type="hidden" name="binding" value=(used.id);
                                input type="hidden" name="back" value=(self_path);
                                label {
                                    "Shown on the card instead of the handler's name"
                                    input type="text" name="title" maxlength="120"
                                        value=(used.title.as_deref().unwrap_or(""))
                                        autocomplete="off" spellcheck="false"
                                        placeholder=(handler.as_deref().unwrap_or("Label title"));
                                }
                                button type="submit" { "Save title" }
                            }
                        }
                    }

                    section class="label-section" {
                        h2 { "Tags" }
                        @if label.tags.is_empty() {
                            p class="empty" { "No tags yet." }
                        } @else {
                            ul class="tag-list" {
                                @for tag in &label.tags {
                                    li {
                                        a href={ "/kiksmiks?tag_" (tag.id) "=1" } { (tag.name) }
                                        form method="post"
                                            action={ "/kiksmiks/" (code) "/tags/" (tag.id) "/remove" } {
                                            input type="hidden" name="back" value=(self_path);
                                            button type="submit" { "Remove" }
                                        }
                                    }
                                }
                            }
                        }

                        @if !available.is_empty() {
                            form class="edit-form" method="post"
                                action={ "/kiksmiks/" (code) "/tags" } {
                                input type="hidden" name="back" value=(self_path);
                                label {
                                    "Existing tag"
                                    select name="id" required {
                                        @for tag in &available {
                                            option value=(tag.id) { (tag.name) }
                                        }
                                    }
                                }
                                button type="submit" { "Add tag" }
                            }
                        }

                        form class="edit-form" method="post"
                            action={ "/kiksmiks/" (code) "/tags" } {
                            input type="hidden" name="back" value=(self_path);
                            label {
                                "New tag"
                                input type="text" name="name" maxlength="40" required
                                    autocomplete="off" spellcheck="false"
                                    placeholder="Loft";
                            }
                            button type="submit" { "Create and add" }
                        }
                    }

                    section class="label-section" x-data="locationTag" {
                        h2 { "Place" }
                        @match &label.place {
                            Some(place) => p {
                                "Tagged as " strong { (place.name) }
                                @if let Some(address) = &place.address {
                                    " — " (address)
                                }
                            },
                            None => p class="empty" { "Not tagged yet." },
                        }

                        @if !places.is_empty() {
                            form class="edit-form" method="post"
                                action={ "/kiksmiks/" (code) "/place" } {
                                input type="hidden" name="back" value=(self_path);
                                label {
                                    "Existing place"
                                    select name="location" required {
                                        @for place in &places {
                                            option value=(place.id) {
                                                (place.name)
                                                @if let Some(address) = &place.address {
                                                    " — " (address)
                                                }
                                            }
                                        }
                                    }
                                }
                                button type="submit" { "Set place" }
                            }
                        }

                        form class="edit-form" method="post"
                            action={ "/kiksmiks/" (code) "/place" } {
                            input type="hidden" name="back" value=(self_path);
                            label {
                                "New place"
                                input type="text" name="name" maxlength="80" required
                                    autocomplete="off" spellcheck="false"
                                    placeholder="Loft, shelf 3";
                            }
                            label {
                                "Address (optional)"
                                input type="text" name="address" maxlength="200"
                                    autocomplete="off" spellcheck="false";
                            }
                            input type="hidden" name="lat" x-model="lat";
                            input type="hidden" name="lon" x-model="lon";

                            div class="locate" {
                                button type="button" x-on:click="locate" {
                                    "Use my location"
                                }
                                span class="hint" x-text="status" {}
                            }
                            button type="submit" disabled x-bind:disabled="noCoords" {
                                "Tag this new place"
                            }
                            noscript {
                                p class="hint" {
                                    "Getting the device's position needs JavaScript. Pick an
                                     existing place instead."
                                }
                            }
                        }

                        @if label.place.is_some() {
                            form class="untag" method="post"
                                action={ "/kiksmiks/" (code) "/place/clear" } {
                                input type="hidden" name="back" value=(self_path);
                                button type="submit" { "Remove place" }
                            }
                        }
                    }

                    @if !history.is_empty() {
                        section class="label-section recover" {
                            h2 { "Previous uses" }
                            p class="hint" {
                                "Recovering one of these keeps what it held, including its
                                 photos."
                            }
                            ul class="recover-list" {
                                @for used in &history {
                                    li {
                                        div class="recover-what" {
                                            strong {
                                                (used.title.as_deref().unwrap_or(&used.handler_id))
                                            }
                                            span class="blurb" {
                                                @if let Some(at) = used.released_at {
                                                    "released " (at.format("%-d %b %Y"))
                                                } @else {
                                                    "released"
                                                }
                                            }
                                        }
                                        @if active.is_none() {
                                            form method="post"
                                                action={ "/kiksmiks/" (code) "/recover" } {
                                                input type="hidden" name="binding" value=(used.id);
                                                input type="hidden" name="back" value=(self_path);
                                                button type="submit" { "Recover" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
        )
        .with_state(json!({ "overview": true })),
    )
    .into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "nothing here\n").into_response()
}

fn bad_request(message: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, message.into()).into_response()
}

fn code_from(raw: &str) -> Option<CodeId> {
    CodeId::parse_checksummed(raw).ok()
}

/// `POST /kiksmiks/{code}/delete` — hide a card, not a sticker.
async fn delete_label(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<Back>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let result = async {
        let mut tx = state.store().begin().await?;
        if !store::soft_delete_code(&mut *tx, &code).await? {
            return Err(qr_core::Error::NotFound("label"));
        }
        store::record_event(
            &mut *tx,
            EventKind::CodeDeleted,
            Some(&code),
            None,
            &json!({}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

/// `POST /kiksmiks/{code}/restore` — show it again.
async fn restore_label(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<Back>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let result = async {
        let mut tx = state.store().begin().await?;
        if !store::restore_code(&mut *tx, &code).await? {
            return Err(qr_core::Error::NotFound("label"));
        }
        store::record_event(
            &mut *tx,
            EventKind::CodeRestored,
            Some(&code),
            None,
            &json!({}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

#[derive(Deserialize)]
struct TitleBody {
    binding: Uuid,
    #[serde(default)]
    title: String,
    #[serde(default)]
    back: String,
}

/// `POST /kiksmiks/{code}/title` — rename a label without opening its capability page.
async fn set_title(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<TitleBody>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };
    let title = body.title.trim();
    if title.chars().count() > 120 {
        return bad_request("a title has at most 120 characters\n");
    }

    let result = async {
        let mut tx = state.store().begin().await?;
        let Some(binding) = store::binding(&mut *tx, &code, body.binding).await? else {
            return Err(qr_core::Error::NotFound("label"));
        };
        store::set_binding_title(&mut *tx, binding.id, (!title.is_empty()).then_some(title))
            .await?;
        store::record_event(
            &mut *tx,
            EventKind::CodeRetitled,
            Some(&code),
            Some(binding.id),
            &json!({ "title": title }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

#[derive(Deserialize)]
struct TagBody {
    #[serde(default)]
    id: Option<Uuid>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    back: String,
}

/// `POST /kiksmiks/{code}/tags` — attach an existing tag, or create one from a name.
async fn add_tag(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<TagBody>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let name = body.name.as_deref().map(str::trim);
    if body.id.is_none() && name.is_some_and(|name| name.is_empty() || name.chars().count() > 40) {
        return bad_request("a tag needs a name of at most 40 characters\n");
    }

    let result = async {
        let mut tx = state.store().begin().await?;
        let tag = match (body.id, name) {
            (Some(id), _) => store::tag(&mut *tx, id)
                .await?
                .ok_or(qr_core::Error::NotFound("tag"))?,
            (None, Some(name)) if !name.is_empty() => store::tag_named(&mut *tx, name).await?,
            (None, _) => {
                return Err(qr_core::Error::Invalid(
                    "an id or a name is needed".to_owned(),
                ));
            }
        };

        store::add_code_tag(&mut *tx, &code, tag.id).await?;
        store::record_event(
            &mut *tx,
            EventKind::TagAdded,
            Some(&code),
            None,
            &json!({ "tag": tag.id, "name": tag.name }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

/// `POST /kiksmiks/{code}/tags/{tag}/remove` — take a tag off this label.
async fn remove_tag(
    State(state): State<AppState>,
    Path((raw, tag)): Path<(String, Uuid)>,
    axum::extract::Form(body): axum::extract::Form<Back>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let result = async {
        let mut tx = state.store().begin().await?;
        store::remove_code_tag(&mut *tx, &code, tag).await?;
        store::record_event(
            &mut *tx,
            EventKind::TagRemoved,
            Some(&code),
            None,
            &json!({ "tag": tag }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

#[derive(Deserialize)]
struct PlaceBody {
    #[serde(default)]
    location: Option<Uuid>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    back: String,
}

/// `POST /kiksmiks/{code}/place` — attach this label to an existing place, or create one.
async fn set_place(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<PlaceBody>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let result = async {
        let mut tx = state.store().begin().await?;

        let place = match body.location {
            Some(id) => store::location(&mut *tx, id)
                .await?
                .ok_or_else(|| qr_core::Error::Invalid("no such place".to_owned()))?,
            None => {
                let name = body.name.as_deref().map(str::trim).unwrap_or_default();
                if name.is_empty() || name.chars().count() > 80 {
                    return Err(qr_core::Error::Invalid(
                        "a place needs a name of at most 80 characters".to_owned(),
                    ));
                }

                let address = body
                    .address
                    .as_deref()
                    .map(str::trim)
                    .filter(|address| !address.is_empty());
                if address.is_some_and(|address| address.chars().count() > 200) {
                    return Err(qr_core::Error::Invalid(
                        "that address is too long".to_owned(),
                    ));
                }

                let (Some(lat), Some(lon)) = (body.lat, body.lon) else {
                    return Err(qr_core::Error::Invalid(
                        "a new place needs coordinates from this device".to_owned(),
                    ));
                };
                if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
                    return Err(qr_core::Error::Invalid(
                        "latitude must be between -90 and 90".to_owned(),
                    ));
                }
                if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
                    return Err(qr_core::Error::Invalid(
                        "longitude must be between -180 and 180".to_owned(),
                    ));
                }

                store::insert_location(
                    &mut *tx,
                    qr_core::NewLocation {
                        name: name.to_owned(),
                        address: address.map(str::to_owned),
                        lat,
                        lon,
                    },
                )
                .await?
            }
        };

        store::set_code_location(&mut *tx, &code, Some(place.id)).await?;
        store::record_event(
            &mut *tx,
            EventKind::LocationTagged,
            Some(&code),
            None,
            &json!({ "location": place.id, "name": place.name }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

/// `POST /kiksmiks/{code}/place/clear` — the label is no longer anywhere in particular.
async fn clear_place(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<Back>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let result = async {
        let mut tx = state.store().begin().await?;
        store::set_code_location(&mut *tx, &code, None).await?;
        store::record_event(
            &mut *tx,
            EventKind::LocationTagged,
            Some(&code),
            None,
            &json!({ "cleared": true }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

/// `POST /kiksmiks/{code}/release` — end the current use, keeping what it held.
async fn release_label(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<Back>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let result = async {
        let mut tx = state.store().begin().await?;
        let Some(binding) = store::active_binding(&mut *tx, &code).await? else {
            return Err(qr_core::Error::NotFound("binding"));
        };
        if let Some(handler) = state.registry().by_name(&binding.handler_id)
            && let Err(error) = handler.on_release(&mut tx, &binding).await
        {
            return Err(qr_core::Error::Invalid(error.to_string()));
        }
        store::release_binding(&mut *tx, binding.id).await?;
        store::record_event(
            &mut *tx,
            EventKind::CodeReleased,
            Some(&code),
            Some(binding.id),
            &json!({}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

#[derive(Deserialize)]
struct RecoverBody {
    binding: Uuid,
    #[serde(default)]
    back: String,
}

/// `POST /kiksmiks/{code}/recover` — put a released use back.
async fn recover_label(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    axum::extract::Form(body): axum::extract::Form<RecoverBody>,
) -> Response {
    let Some(code) = code_from(&raw) else {
        return not_found();
    };

    let result = async {
        let mut tx = state.store().begin().await?;
        let Some(binding) = store::binding(&mut *tx, &code, body.binding).await? else {
            return Err(qr_core::Error::NotFound("binding"));
        };
        if binding.released_at.is_none() {
            return Err(qr_core::Error::Conflict("binding"));
        }
        if store::active_binding(&mut *tx, &code).await?.is_some() {
            return Err(qr_core::Error::Conflict("binding"));
        }
        if !store::recover_binding(&mut *tx, binding.id).await? {
            return Err(qr_core::Error::Conflict("binding"));
        }
        store::record_event(
            &mut *tx,
            EventKind::CodeRecovered,
            Some(&code),
            Some(binding.id),
            &json!({ "handler": binding.handler_id }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => redirect(&back_to(&body.back)),
        Err(error) => store_response(&error),
    }
}

fn redirect(location: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, location.to_owned())],
    )
        .into_response()
}

fn store_response(error: &qr_core::Error) -> Response {
    if error.is_not_found() {
        return (StatusCode::NOT_FOUND, "nothing here\n").into_response();
    }
    if error.is_conflict() {
        return (StatusCode::CONFLICT, "that already happened\n").into_response();
    }
    if error.is_invalid() {
        return (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response();
    }
    tracing::error!(%error, "an overview request could not reach the database");
    (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response()
}

/// `GET /kiksmiks/photo/{photo}/thumb` — the small copy, without a code cookie.
async fn thumb(
    State(state): State<AppState>,
    Path(photo): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let key = match qr_photos::store::thumb_key(state.store().pool(), photo).await {
        Ok(Some(key)) => key,
        Ok(None) => return (StatusCode::NOT_FOUND, "nothing here\n").into_response(),
        Err(error) => return store_response(&error),
    };

    let etag = format!("\"{key}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag))
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let Ok(parsed) = BlobKey::parse(&key) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong\n").into_response();
    };
    let blob = match state.blobs().open(&parsed).await {
        Ok(blob) => blob,
        Err(error) => return store_response(&error),
    };

    let mut response =
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(blob.reader))
            .into_response();
    let headers = response.headers_mut();
    if let Ok(value) = header::HeaderValue::from_str("image/jpeg") {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = header::HeaderValue::from_str(&blob.byte_size.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Ok(value) = header::HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    if let Ok(value) = header::HeaderValue::from_str("public, max-age=31536000, immutable") {
        headers.insert(header::CACHE_CONTROL, value);
    }
    response
}
