//! Refuses every top-level navigation of the webview away from the app's own origin.
//!
//! The ACL already refuses all commands for a remote origin, so this is defence in depth: a page
//! that is not ours must not even be shown inside the main window. Terminal links do not navigate;
//! they go through the `open_external_url` command, which opens them in Rust.

use tauri::Url;

/// True on the platforms where Tauri serves the app from `http(s)://tauri.localhost` (Windows and
/// Android); everywhere else it is `tauri://localhost` (see `tauri::protocol::origin`).
const HTTP_ORIGIN_PLATFORM: bool = cfg!(any(windows, target_os = "android"));

/// The origin the release webview serves the app from on one platform: `http(s)://tauri.localhost`
/// when `http_origin` is set (Windows and Android), `tauri://localhost` otherwise. Only the
/// platform's own origin is allowed, so e.g. a loopback `http://tauri.localhost/` is not shown on
/// macOS or Linux.
fn is_release_origin_for(url: &Url, http_origin: bool) -> bool {
    match (url.scheme(), url.host_str()) {
        ("http" | "https", Some("tauri.localhost")) => http_origin,
        ("tauri", Some("localhost")) => !http_origin,
        _ => false,
    }
}

fn is_release_origin(url: &Url) -> bool {
    is_release_origin_for(url, HTTP_ORIGIN_PLATFORM)
}

/// True when `url` is the same origin (scheme, host and port) as the dev server.
fn is_dev_origin(url: &Url, dev_url: &Url) -> bool {
    url.scheme() == dev_url.scheme()
        && url.host_str().is_some()
        && url.host_str() == dev_url.host_str()
        && url.port_or_known_default() == dev_url.port_or_known_default()
}

/// Decides whether the main webview may navigate to `url`.
///
/// Allowed: the platform's release origin (any path, query and fragment, but no non-default port
/// and no userinfo) and, in a dev build, exactly the origin of `build.devUrl`. Everything else is
/// refused.
#[must_use]
pub fn is_app_url(url: &Url, dev: bool, dev_url: Option<&Url>) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.port().is_none() && is_release_origin(url) {
        return true;
    }
    dev && dev_url.is_some_and(|d| is_dev_origin(url, d))
}

/// The plugin that installs the guard.
#[must_use]
pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("fernjo-nav-guard")
        .on_navigation(|webview, url| {
            use tauri::Manager;
            is_app_url(
                url,
                tauri::is_dev(),
                webview.config().build.dev_url.as_ref(),
            )
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn dev_url() -> Url {
        u("http://localhost:1420")
    }

    const HTTP_FORMS: [&str; 6] = [
        "http://tauri.localhost/",
        "http://tauri.localhost/index.html#x",
        "http://tauri.localhost/a/b?q=1#f",
        "https://tauri.localhost/",
        "HTTP://TAURI.LOCALHOST/",
        // the parser decodes the host, so this is the very same host
        "http://tauri%2Elocalhost/",
    ];
    const TAURI_FORMS: [&str; 2] = ["tauri://localhost", "tauri://localhost/index.html"];

    #[test]
    fn windows_and_android_allow_only_the_http_origins() {
        for s in HTTP_FORMS {
            assert!(is_release_origin_for(&u(s), true), "{s}");
        }
        for s in TAURI_FORMS {
            assert!(!is_release_origin_for(&u(s), true), "{s}");
        }
    }

    #[test]
    fn macos_and_linux_allow_only_the_tauri_origin() {
        for s in TAURI_FORMS {
            assert!(is_release_origin_for(&u(s), false), "{s}");
        }
        for s in HTTP_FORMS {
            assert!(!is_release_origin_for(&u(s), false), "{s}");
        }
    }

    #[test]
    fn the_running_platform_allows_exactly_its_own_origin() {
        let (own, foreign) = if cfg!(any(windows, target_os = "android")) {
            (HTTP_FORMS.as_slice(), TAURI_FORMS.as_slice())
        } else {
            (TAURI_FORMS.as_slice(), HTTP_FORMS.as_slice())
        };
        for s in own {
            assert!(is_app_url(&u(s), false, Some(&dev_url())), "{s}");
            assert!(is_app_url(&u(s), false, None), "{s}");
        }
        for s in foreign {
            assert!(!is_app_url(&u(s), false, Some(&dev_url())), "{s}");
            assert!(!is_app_url(&u(s), false, None), "{s}");
        }
    }

    #[test]
    fn dev_origin_is_allowed_only_in_dev() {
        let d = dev_url();
        assert!(is_app_url(&u("http://localhost:1420/x"), true, Some(&d)));
        assert!(is_app_url(&u("http://localhost:1420"), true, Some(&d)));
        assert!(!is_app_url(&u("http://localhost:1420/"), false, Some(&d)));
        assert!(!is_app_url(&u("http://localhost:1420/"), true, None));
    }

    #[test]
    fn everything_else_is_refused() {
        let d = dev_url();
        for s in [
            "http://127.0.0.1:8099/",
            "http://localhost:1421/",
            "http://localhost.evil.com:1420",
            "http://localhost:1420@evil.com/",
            "http://evil.com@localhost:1420/",
            "https://localhost:1420/",
            "http://localhost/",
            "http://tauri.localhost:8080/",
            "https://tauri.localhost:80/",
            "http://tauri.localhost:80@evil.com/",
            "https://tauri.localhost@evil.com/",
            "http://tauri.localhost@evil.com/",
            "http://user:pw@tauri.localhost/",
            "http://tauri.localhost.evil.com/",
            "http://tauri.localhost./",
            "http://evil.tauri.localhost/",
            "http://xn--tauri-localhost.com/",
            "tauri://localhost:8080/",
            "tauri://evil/",
            "tauri://user@localhost/",
            "ftp://tauri.localhost/",
            "ws://tauri.localhost/",
            "file:///C:/x",
            "javascript:alert(1)",
            "data:text/html,x",
            "blob:http://tauri.localhost/1234",
            "about:blank",
            "https://example.com/",
        ] {
            assert!(!is_app_url(&u(s), true, Some(&d)), "dev: {s}");
            assert!(!is_app_url(&u(s), false, Some(&d)), "release: {s}");
        }
    }
}
