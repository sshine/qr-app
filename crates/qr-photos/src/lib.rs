//! What is in the box.
//!
//! Photographing a storage box is the whole reason this service exists, and the album
//! is deliberately the plainest thing that does it: a grid, newest first, a camera
//! button, and an undo for when the wrong one is deleted.

pub mod pipeline;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use maud::{Markup, html};
use qr_core::{Binding, BlobKey, model::EventKind, store as core_store};
use qr_handler::{
    AppState, ChooserCard, CodeCtx, Handler, HandlerCtx, HandlerError, HandlerId, HandlerPage,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{pipeline::Processed, store::Photo};

const ICON: &str = r##"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M3 9a2 2 0 0 1 2-2h2l1.5-2h7L17 7h2a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><circle cx="12" cy="13" r="3.5"/></svg>"##;

/// How many photos one page of the album holds.
const PAGE: i64 = 60;

pub struct PhotoAlbum;

#[async_trait]
impl Handler for PhotoAlbum {
    fn id(&self) -> HandlerId {
        HandlerId::from_static("photo-album")
    }

    fn name(&self) -> &'static str {
        "Photos"
    }

    fn chooser(&self) -> ChooserCard {
        ChooserCard {
            title: "Photos",
            blurb: "Photograph what is in the box, and look at it again later.",
            icon: maud::PreEscaped(ICON),
        }
    }

    async fn describe(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<serde_json::Value, HandlerError> {
        let photos = store::list(ctx.store().pool(), binding.id, None, PAGE).await?;
        let total = store::count(ctx.store().pool(), binding.id).await?;
        Ok(json!({
            "photos": photos.iter().map(|p| as_json(&binding.code_id, p)).collect::<Vec<_>>(),
            "total": total,
            "page": PAGE,
            "maxBytes": ctx.limits().max_upload_bytes,
        }))
    }

    async fn render(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<HandlerPage, HandlerError> {
        let photos = store::list(ctx.store().pool(), binding.id, None, PAGE).await?;
        Ok(HandlerPage::Markup(album(binding, &photos)))
    }

    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/photos", get(list_photos).post(upload))
            .route(
                "/photos/{photo}",
                get(read_photo).patch(caption).delete(remove),
            )
            .route("/photos/{photo}/restore", post(restore))
            .route("/photos/{photo}/{size}", get(serve_blob))
            .route("/trash", get(read_trash))
            // Only this subtree may receive a photo. The global limit stays small, so
            // nothing else can be made to buffer megabytes by asking.
            .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
    }
}

/// One photo, as the API and the hydration payload see it.
fn as_json(code: &qr_core::CodeId, photo: &Photo) -> serde_json::Value {
    json!({
        "id": photo.id,
        "thumb": media_url(code, photo.id, "thumb"),
        "view": media_url(code, photo.id, "view"),
        "full": media_url(code, photo.id, "full"),
        "width": photo.width,
        "height": photo.height,
        "caption": photo.caption,
        "takenAt": photo.taken_at,
        "createdAt": photo.created_at,
        "deletedAt": photo.deleted_at,
    })
}

/// Under the code's own path, so the capability cookie reaches an `<img>` as readily as
/// it reaches a fetch.
fn media_url(code: &qr_core::CodeId, photo: Uuid, size: &str) -> String {
    format!("/c/{code}/api/v1/h/photo-album/photos/{photo}/{size}")
}

fn error_response(error: &qr_core::Error) -> Response {
    if error.is_not_found() {
        return (StatusCode::NOT_FOUND, "nothing here\n").into_response();
    }
    if error.is_invalid() {
        return (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response();
    }
    tracing::error!(%error, "an album request failed");
    (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response()
}

/// A code reached the album without being bound to it, which only happens if it was
/// released between the capability check and here.
fn not_claimed() -> Response {
    (StatusCode::NOT_FOUND, "not claimed\n").into_response()
}

#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    before: Option<Uuid>,
}

async fn list_photos(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Response {
    let Some(binding) = access.binding.as_ref() else {
        return not_claimed();
    };

    match store::list(state.store().pool(), binding.id, page.before, PAGE).await {
        Ok(photos) => Json(json!({
            "photos": photos.iter().map(|p| as_json(&access.code.id, p)).collect::<Vec<_>>(),
            // Null when the page was not full, so a client knows to stop asking.
            "next": (photos.len() as i64 == PAGE)
                .then(|| photos.last().map(|p| p.id))
                .flatten(),
        }))
        .into_response(),
        Err(error) => error_response(&error),
    }
}

async fn read_trash(CodeCtx(access): CodeCtx, State(state): State<AppState>) -> Response {
    let Some(binding) = access.binding.as_ref() else {
        return not_claimed();
    };
    match store::trash(state.store().pool(), binding.id).await {
        Ok(photos) => Json(json!({
            "photos": photos.iter().map(|p| as_json(&access.code.id, p)).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => error_response(&error),
    }
}

async fn read_photo(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Path((_code, photo)): Path<(String, Uuid)>,
) -> Response {
    let Some(binding) = access.binding.as_ref() else {
        return not_claimed();
    };
    match store::get(state.store().pool(), binding.id, photo).await {
        Ok(Some(photo)) => Json(as_json(&access.code.id, &photo)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "nothing here\n").into_response(),
        Err(error) => error_response(&error),
    }
}

/// Takes one or more photos from a multipart form.
///
/// The same endpoint answers a plain form post with a redirect and a fetch with JSON,
/// so the album works with script and without it from one code path.
async fn upload(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let Some(binding) = access.binding.clone() else {
        return not_claimed();
    };

    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));

    let limits = pipeline::Limits {
        max_bytes: state.limits().max_upload_bytes,
        max_pixels: state.limits().max_upload_pixels,
        strip_original: state.limits().strip_original_exif,
    };

    let mut added = Vec::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response();
            }
        };

        let Ok(bytes) = field.bytes().await else {
            return (
                StatusCode::BAD_REQUEST,
                "that upload did not arrive whole\n",
            )
                .into_response();
        };
        if bytes.is_empty() {
            continue;
        }

        // Decoding and two resizes are hundreds of milliseconds of pure CPU, which
        // would otherwise stall every other request sharing this runtime thread.
        let processed = match tokio::task::spawn_blocking({
            let limits = pipeline::Limits { ..limits };
            move || pipeline::process(bytes, &limits)
        })
        .await
        {
            Ok(Ok(processed)) => processed,
            Ok(Err(error)) => {
                return (StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("{error}\n")).into_response();
            }
            Err(error) => {
                tracing::error!(%error, "the image pipeline panicked");
                return (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong\n")
                    .into_response();
            }
        };

        match persist(&state, &binding, processed).await {
            Ok(photo) => added.push(photo),
            Err(error) => return error_response(&error),
        }
    }

    if added.is_empty() {
        return (StatusCode::BAD_REQUEST, "no photo arrived\n").into_response();
    }

    if wants_json {
        (
            StatusCode::CREATED,
            Json(json!({
                "photos": added.iter().map(|p| as_json(&access.code.id, p)).collect::<Vec<_>>(),
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::SEE_OTHER,
            [(header::LOCATION, format!("/c/{}", access.code.id))],
        )
            .into_response()
    }
}

/// Writes the blobs, then the row.
///
/// In that order, because a row pointing at a blob that is not there is a broken album
/// and a blob with no row is something the collector sweeps.
async fn persist(
    state: &AppState,
    binding: &Binding,
    processed: Processed,
) -> qr_core::Result<Photo> {
    let blobs = Arc::clone(state.blobs());

    let original = BlobKey::of(&processed.original);
    let view = BlobKey::of(&processed.view);
    let thumb = BlobKey::of(&processed.thumb);

    blobs.put(&original, processed.original.clone()).await?;
    blobs.put(&view, processed.view.clone()).await?;
    blobs.put(&thumb, processed.thumb.clone()).await?;

    let mut tx = state.store().begin().await?;
    for (key, bytes, media_type) in [
        (&original, processed.original.len(), processed.media_type),
        (&view, processed.view.len(), "image/jpeg"),
        (&thumb, processed.thumb.len(), "image/jpeg"),
    ] {
        core_store::register_blob(
            &mut *tx,
            &key.as_hex(),
            i64::try_from(bytes).unwrap_or(i64::MAX),
            media_type,
        )
        .await?;
    }

    let photo = store::insert(
        &mut *tx,
        store::NewPhoto {
            binding_id: binding.id,
            original_key: &original.as_hex(),
            view_key: &view.as_hex(),
            thumb_key: &thumb.as_hex(),
            media_type: processed.media_type,
            width: i32::try_from(processed.width).unwrap_or(i32::MAX),
            height: i32::try_from(processed.height).unwrap_or(i32::MAX),
            byte_size: i64::try_from(processed.original.len()).unwrap_or(i64::MAX),
            taken_at: processed.taken_at,
        },
    )
    .await?;

    core_store::record_event(
        &mut *tx,
        EventKind::PhotoAdded,
        Some(&binding.code_id),
        Some(binding.id),
        &json!({ "photo": photo.id }),
    )
    .await?;
    tx.commit().await?;

    Ok(photo)
}

#[derive(Deserialize)]
pub struct CaptionBody {
    caption: Option<String>,
}

async fn caption(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Path((_code, photo)): Path<(String, Uuid)>,
    Json(body): Json<CaptionBody>,
) -> Response {
    let Some(binding) = access.binding.as_ref() else {
        return not_claimed();
    };
    let caption = body
        .caption
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    if caption.is_some_and(|c| c.chars().count() > 280) {
        return (StatusCode::BAD_REQUEST, "that caption is too long\n").into_response();
    }

    match store::set_caption(state.store().pool(), binding.id, photo, caption).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "nothing here\n").into_response(),
        Err(error) => error_response(&error),
    }
}

/// Soft, and that is the whole design: the undo is the confirmation, so there is no
/// dialog to tap through on a phone for something already reversible.
async fn remove(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Path((_code, photo)): Path<(String, Uuid)>,
) -> Response {
    let Some(binding) = access.binding.clone() else {
        return not_claimed();
    };

    match store::soft_delete(state.store().pool(), binding.id, photo).await {
        Ok(true) => {
            let _ = core_store::record_event(
                state.store().pool(),
                EventKind::PhotoDeleted,
                Some(&access.code.id),
                Some(binding.id),
                &json!({ "photo": photo }),
            )
            .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "nothing here\n").into_response(),
        Err(error) => error_response(&error),
    }
}

async fn restore(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Path((_code, photo)): Path<(String, Uuid)>,
) -> Response {
    let Some(binding) = access.binding.clone() else {
        return not_claimed();
    };

    match store::restore(state.store().pool(), binding.id, photo).await {
        Ok(true) => {
            let _ = core_store::record_event(
                state.store().pool(),
                EventKind::PhotoRestored,
                Some(&access.code.id),
                Some(binding.id),
                &json!({ "photo": photo }),
            )
            .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "nothing here\n").into_response(),
        Err(error) => error_response(&error),
    }
}

/// Serves one of the three sizes.
///
/// Content-addressed, so the ETag is the content and `immutable` is simply true. The
/// cache directive is `private` because the bytes are capability-protected and must not
/// sit in anything shared, and `Vary: Cookie` keeps a browser's own copy keyed to the
/// cookie that fetched it.
async fn serve_blob(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Path((_code, photo, size)): Path<(String, Uuid, String)>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(binding) = access.binding.as_ref() else {
        return not_claimed();
    };

    let found = match store::get(state.store().pool(), binding.id, photo).await {
        Ok(Some(found)) => found,
        Ok(None) => return (StatusCode::NOT_FOUND, "nothing here\n").into_response(),
        Err(error) => return error_response(&error),
    };

    let (key, media_type, download) = match size.as_str() {
        "thumb" => (&found.thumb_key, "image/jpeg", false),
        "view" => (&found.view_key, "image/jpeg", false),
        "full" => (&found.original_key, found.media_type.as_str(), true),
        _ => return (StatusCode::NOT_FOUND, "no such size\n").into_response(),
    };

    let etag = format!("\"{key}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|candidate| candidate.trim() == etag))
    {
        // Answered before the file is even opened.
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let Ok(parsed) = BlobKey::parse(key) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong\n").into_response();
    };
    let blob = match state.blobs().open(&parsed).await {
        Ok(blob) => blob,
        Err(error) => return error_response(&error),
    };

    let mut response =
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(blob.reader))
            .into_response();

    let headers = response.headers_mut();
    insert_header(headers, header::CONTENT_TYPE, media_type);
    insert_header(headers, header::CONTENT_LENGTH, &blob.byte_size.to_string());
    insert_header(headers, header::ETAG, &etag);
    insert_header(
        headers,
        header::CACHE_CONTROL,
        "private, max-age=31536000, immutable",
    );
    insert_header(headers, header::VARY, "Cookie");
    if download {
        insert_header(
            headers,
            header::CONTENT_DISPOSITION,
            &format!("attachment; filename=\"{photo}.jpg\""),
        );
    }

    response
}

fn insert_header(headers: &mut axum::http::HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(value) = header::HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

/// The album.
///
/// Every control here is a form that works without script. The Alpine component on top
/// of it submits the camera's output without a second tap, shows the new photo before
/// the upload finishes, and turns a delete into an undo.
fn album(binding: &Binding, photos: &[Photo]) -> Markup {
    let code = &binding.code_id;
    let upload_action = format!("/c/{code}/api/v1/h/photo-album/photos");

    html! {
        main class="album" x-data="album" {
            header class="album-head" {
                h1 { (binding.title.as_deref().unwrap_or("What is in the box")) }
                p class="code-id" { (code.grouped()) }
            }

            form class="capture" method="post" action=(upload_action)
                enctype="multipart/form-data" x-ref="form"
                "x-on:change"="maybeSubmit" {
                label class="capture-button" {
                    // capture hands off to the phone's own camera app: autofocus, HDR,
                    // flash, the volume-button shutter and a retake screen, none of
                    // which is worth reimplementing in a viewfinder nobody asked for.
                    input type="file" name="photo" accept="image/*"
                        capture="environment" hidden;
                    span { "Take a photo" }
                }
                label class="pick-button" {
                    // capture and multiple are mutually exclusive in practice, so
                    // choosing several needs a control of its own.
                    input type="file" name="photo" accept="image/*" multiple hidden;
                    span { "Choose photos" }
                }
                noscript {
                    button type="submit" { "Upload" }
                }
            }

            @if photos.is_empty() {
                p class="empty" { "Nothing photographed yet." }
            }

            ul class="grid" {
                @for photo in photos {
                    li class="tile" {
                        a href=(media_url(code, photo.id, "view")) {
                            img src=(media_url(code, photo.id, "thumb"))
                                width=(photo.width) height=(photo.height)
                                loading="lazy" decoding="async"
                                alt=(photo.caption.as_deref().unwrap_or("a photo of the contents"));
                        }
                        form method="post"
                            action={ (media_url(code, photo.id, "")) "delete" }
                            class="tile-delete" {
                            button type="submit" aria-label="Delete" { "×" }
                        }
                    }
                }
            }

            footer class="album-foot" {
                a href={ "/c/" (code) "/api/v1/h/photo-album/trash" } { "Deleted photos" }
            }
        }
    }
}
