//! Keep the popup document in place even when navigation bypasses DOM click handlers.
use tauri::{Config, Url};

pub fn document_url(config: &Config, route: &str) -> tauri::Result<Url> {
    let base = if tauri::is_dev() {
        config.build.dev_url.clone()
    } else {
        None
    }
    .unwrap_or_else(|| {
        Url::parse(if cfg!(target_os = "windows") {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        })
        .expect("valid app origin")
    });
    base.join(route).map_err(tauri::Error::InvalidUrl)
}

pub fn allows(document: &Url, destination: &Url) -> bool {
    // Wry also reports subframe navigation on macOS. Mermaid uses sandboxed, script-free
    // data documents, and empty frames can load about:blank before their final document.
    if destination.as_str() == "about:blank"
        || destination
            .as_str()
            .starts_with("data:text/html;charset=UTF-8;base64,")
    {
        return true;
    }
    // Keep the complete route, including view/effect query parameters. Allowing every
    // app-origin path would admit Tauri's SPA fallback and recreate the broken popup.
    let mut destination = destination.clone();
    destination.set_fragment(None);
    &destination == document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_original_popup_route_can_load() {
        for base in [
            "tauri://localhost/",
            "http://tauri.localhost/",
            "http://localhost:5180/",
        ] {
            for view in ["popup", "popup-inbox"] {
                let document = Url::parse(base)
                    .unwrap()
                    .join(&format!("index.html?view={view}&effect=solid"))
                    .unwrap();
                assert!(allows(&document, &document));
                assert!(allows(&document, &document.join("#heading").unwrap()));
                for target in [
                    "/Users/test/My%20Report.md:82:5",
                    "docs/report.md#L8",
                    "index.html",
                    "?view=settings",
                    "?view=popup-inbox",
                    "https://example.com/",
                    "file:///Users/test/report.md",
                    "data:text/html,<h1>file</h1>",
                ] {
                    assert!(
                        !allows(&document, &document.join(target).unwrap()),
                        "unexpected navigation from {document} to {target}"
                    );
                }
            }
        }
    }

    #[test]
    fn sandboxed_diagram_documents_still_load() {
        let document = Url::parse("tauri://localhost/index.html?view=popup-inbox").unwrap();
        for frame in [
            "about:blank",
            "data:text/html;charset=UTF-8;base64,PHN2Zz48L3N2Zz4=",
        ] {
            assert!(allows(&document, &Url::parse(frame).unwrap()));
        }
    }
}
