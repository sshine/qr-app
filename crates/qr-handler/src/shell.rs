//! The page every handler's markup is wrapped in.
//!
//! Wrapped rather than composed by each handler, so none of them can forget the
//! viewport, the manifest, the stylesheet or the hydration payload. The payload is what
//! makes a cold scan render in one round trip: the server already read the state to
//! render the HTML, so sending it twice — once as markup, once as JSON — costs a few
//! hundred bytes and saves a request and a spinner.

use maud::{DOCTYPE, Markup, PreEscaped, html};

pub struct Page {
    pub title: String,
    /// Picked up by `Alpine.data` in `init()`, so no fetch is needed to draw the first
    /// screen.
    pub state: serde_json::Value,
    /// What the handler drew.
    pub body: Markup,
    /// Anything extra the handler wants in `<head>`.
    pub head: Markup,
}

impl Page {
    pub fn new(title: impl Into<String>, body: Markup) -> Self {
        Self {
            title: title.into(),
            state: serde_json::Value::Null,
            body,
            head: html! {},
        }
    }

    #[must_use]
    pub fn with_state(mut self, state: serde_json::Value) -> Self {
        self.state = state;
        self
    }

    #[must_use]
    pub fn with_head(mut self, head: Markup) -> Self {
        self.head = head;
        self
    }
}

/// Serialises a value into a `<script type="application/json">` body.
///
/// `<` is escaped to its JSON unicode form, which parsers accept unchanged and which
/// makes a `</script>` inside any string impossible to write. maud escapes text
/// everywhere else, but the contents of a script element are raw by definition.
pub fn json_script(value: &serde_json::Value) -> PreEscaped<String> {
    let encoded = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
    PreEscaped(encoded.replace('<', "\\u003c"))
}

/// The id the client reads the hydration payload from.
pub const STATE_ID: &str = "qr-state";

pub fn render(page: Page) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                // viewport-fit=cover so a full-bleed photo grid reaches the edges on a
                // phone with rounded corners; the stylesheet puts the safe-area insets
                // back where they matter.
                meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover";
                meta name="color-scheme" content="light dark";
                meta name="theme-color" content="#111111";
                title { (page.title) }
                link rel="manifest" href="/manifest.webmanifest";
                link rel="icon" href="/assets/icon.svg" type="image/svg+xml";
                link rel="stylesheet" href="/assets/app.css";
                (page.head)
            }
            body {
                (page.body)
                script type="application/json" id=(STATE_ID) { (json_script(&page.state)) }
                // app.js first: it registers every Alpine.data component from an
                // alpine:init listener, and deferred scripts run in document order, so
                // Alpine starting first would fire that event before anything is
                // listening and leave every x-data naming an undefined component.
                script src="/assets/app.js" defer {}
                script src="/assets/alpine.js" defer {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn no_angle_bracket_survives_into_the_payload() {
        // The property, not the spelling: asserting on the escape sequence would pass
        // for a replacement that does nothing, which is exactly what this once was.
        let hostile = json!({ "caption": "</script><script>alert(1)</script>" });
        let encoded = json_script(&hostile).0;
        assert!(
            !encoded.contains('<'),
            "a raw < reached the script body: {encoded}"
        );
    }

    #[test]
    fn the_payload_is_still_json_after_escaping() {
        let value = json!({ "a": "<b>", "n": 1 });
        let encoded = json_script(&value).0;
        let parsed: serde_json::Value = serde_json::from_str(&encoded).expect("still json");
        assert_eq!(parsed, value);
    }

    #[test]
    fn the_shell_supplies_what_a_handler_would_forget() {
        let markup = render(Page::new("a box", html! { p { "hello" } }));
        let page = markup.into_string();
        for expected in [
            "<!DOCTYPE html>",
            "viewport",
            "manifest.webmanifest",
            "/assets/app.css",
            "/assets/alpine.js",
            STATE_ID,
        ] {
            assert!(page.contains(expected), "the shell is missing {expected}");
        }
    }
}
