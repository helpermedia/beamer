//! Where the web UI of a plugin comes from.
//!
//! The export macro embeds what [`find`] returns and the bundler decides
//! by it whether the plugin has an editor, so that the two always agree.

use std::path::Path;

/// Environment variable with the URL of a dev server that serves the web
/// UI in place of embedded files.
pub const DEV_URL_VAR: &str = "BEAMER_DEV_URL";

/// Environment variable the bundler sets to the [`stamp`] of the web UI it
/// found. The expanded export macro names it, so that the plugin is
/// expanded again when the web UI's files come, go or change places.
pub const STAMP_VAR: &str = "BEAMER_WEB_UI_STAMP";

/// The web UI of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebUi {
    /// Loaded from a dev server at this URL.
    DevServer(String),
    /// Files embedded in the binary.
    Embedded {
        /// The folder of the files, relative to the package: `webview` or
        /// `webview/dist`.
        dir: &'static str,
        /// The paths of the files, relative to `dir`, with `/` between
        /// folders, sorted.
        files: Vec<String>,
    },
}

/// The web UI of the package in `package_dir`, if it has one.
///
/// In this order:
/// 1. `dev_url` (the value of [`DEV_URL_VAR`]): a dev server.
/// 2. `webview/package.json` and a non-empty `webview/dist/`: the files of
///    `webview/dist/`. Without `dist/` the web build has not run yet and
///    the plugin has no web UI.
/// 3. A non-empty `webview/` without `package.json`: the files of
///    `webview/`.
///
/// Dotfiles, dot folders, `node_modules` and source maps are left out.
pub fn find(package_dir: &Path, dev_url: Option<String>) -> Result<Option<WebUi>, String> {
    if let Some(url) = dev_url {
        return Ok(Some(WebUi::DevServer(url)));
    }
    let webview_dir = package_dir.join("webview");
    let (dir, scan_dir) = if webview_dir.join("package.json").exists() {
        ("webview/dist", webview_dir.join("dist"))
    } else {
        ("webview", webview_dir)
    };
    if !scan_dir.is_dir() {
        return Ok(None);
    }
    let mut files = Vec::new();
    scan(&scan_dir, &scan_dir, &mut files)
        .map_err(|error| format!("failed to scan {}: {}", scan_dir.display(), error))?;
    if files.is_empty() {
        return Ok(None);
    }
    files.sort();
    Ok(Some(WebUi::Embedded { dir, files }))
}

/// A short text that changes whenever what [`find`] returned changes,
/// except for the contents of the files.
pub fn stamp(web_ui: Option<&WebUi>) -> String {
    let description = match web_ui {
        None => "none".to_string(),
        Some(WebUi::DevServer(url)) => format!("dev {url}"),
        Some(WebUi::Embedded { dir, files }) => format!("{dir}\n{}", files.join("\n")),
    };
    format!("{:08x}", crate::fnv1a_32(&description))
}

fn scan(base: &Path, dir: &Path, files: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            scan(base, &path, files)?;
        } else if path.is_file() && !name.ends_with(".map") {
            let relative = path.strip_prefix(base).unwrap_or(&path);
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    // A fresh, empty package folder for one test.
    fn package(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("beamer-web-ui-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: PathBuf) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    #[test]
    fn test_no_webview_folder_is_no_web_ui() {
        assert_eq!(find(&package("none"), None), Ok(None));
    }

    #[test]
    fn test_dev_url_comes_first() {
        let dir = package("dev");
        write(dir.join("webview/index.html"));
        let url = "http://localhost:5173".to_string();
        assert_eq!(find(&dir, Some(url.clone())), Ok(Some(WebUi::DevServer(url))));
    }

    #[test]
    fn test_plain_folder_is_embedded_without_what_is_left_out() {
        let dir = package("plain");
        write(dir.join("webview/index.html"));
        write(dir.join("webview/js/app.js"));
        write(dir.join("webview/js/app.js.map"));
        write(dir.join("webview/.hidden"));
        write(dir.join("webview/node_modules/lib.js"));
        let expected = WebUi::Embedded {
            dir: "webview",
            files: vec!["index.html".to_string(), "js/app.js".to_string()],
        };
        assert_eq!(find(&dir, None), Ok(Some(expected)));
    }

    #[test]
    fn test_web_build_without_dist_is_no_web_ui() {
        let dir = package("unbuilt");
        write(dir.join("webview/package.json"));
        write(dir.join("webview/index.html"));
        assert_eq!(find(&dir, None), Ok(None));
    }

    #[test]
    fn test_web_build_embeds_dist() {
        let dir = package("built");
        write(dir.join("webview/package.json"));
        write(dir.join("webview/index.html"));
        write(dir.join("webview/dist/index.html"));
        let expected = WebUi::Embedded { dir: "webview/dist", files: vec!["index.html".to_string()] };
        assert_eq!(find(&dir, None), Ok(Some(expected)));
    }

    #[test]
    fn test_empty_folder_is_no_web_ui() {
        let dir = package("empty");
        fs::create_dir_all(dir.join("webview/dist")).unwrap();
        assert_eq!(find(&dir, None), Ok(None));
        write(dir.join("webview/package.json"));
        assert_eq!(find(&dir, None), Ok(None));
    }

    #[test]
    fn test_stamp_changes_with_the_files() {
        let one = WebUi::Embedded { dir: "webview", files: vec!["index.html".to_string()] };
        let two = WebUi::Embedded {
            dir: "webview",
            files: vec!["index.html".to_string(), "app.js".to_string()],
        };
        assert_ne!(stamp(Some(&one)), stamp(Some(&two)));
        assert_ne!(stamp(Some(&one)), stamp(None));
        assert_eq!(stamp(Some(&one)), stamp(Some(&one.clone())));
    }
}
