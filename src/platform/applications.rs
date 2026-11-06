use super::*;
use objc2_app_kit::NSWorkspace;
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct OpenApplication {
    pub path: PathBuf,
    pub name: String,
    pub is_default: bool,
}

pub fn applications_for_file(
    path: &Path,
    cancel: &Cancellation,
) -> io::Result<Vec<OpenApplication>> {
    objc2::rc::autoreleasepool(|_| {
        cancel.check()?;
        let workspace = NSWorkspace::sharedWorkspace();
        let url = file_url(path);
        let default = workspace
            .URLForApplicationToOpenURL(&url)
            .filter(|u| u.isFileURL())
            .map(|u| url_path(&u));
        let mut paths = Vec::new();
        if let Some(path) = &default {
            paths.push(path.clone());
        }
        for url in workspace.URLsForApplicationsToOpenURL(&url).iter() {
            cancel.check()?;
            if url.isFileURL() {
                paths.push(url_path(&url));
            }
        }
        let mut seen = HashSet::new();
        let mut result = vec![];
        for path in paths {
            cancel.check()?;
            if !seen.insert(path.clone()) {
                continue;
            }
            // LaunchServices already supplied the candidates. Do not probe
            // every application volume just to populate this chooser.
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            result.push(OpenApplication {
                is_default: default.as_ref() == Some(&path),
                path,
                name: name.strip_suffix(".app").unwrap_or(&name).to_owned(),
            });
        }
        result.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(result)
    })
}
