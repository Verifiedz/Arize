//! The built-in workspace templates (ADR 0025 §6), embedded in the binary so they work with no
//! files. Each is a folder in `crates/modules/workspaces/templates/`; the shared helper in
//! `templates/lib/` is copied into every one.

use shimmer_core::{Error, Result};

use crate::template::Template;

macro_rules! file {
    ($path:literal, $from:literal) => {
        ($path, include_str!(concat!("../templates/", $from)))
    };
}

const HELPER: (&str, &str) = file!("lib/shimmer-open.sh", "lib/shimmer-open.sh");
const VM_HELPER: (&str, &str) = file!("lib/shimmer-vm.sh", "lib/shimmer-vm.sh");

/// `(id, files)`, sorted by id.
const BUILTINS: &[(&str, &[(&str, &str)])] = &[
    (
        "smoke-test",
        &[
            file!("template.toml", "smoke-test/template.toml"),
            file!("workspace.toml", "smoke-test/workspace.toml"),
            file!("steps/01-check.sh", "smoke-test/steps/01-check.sh"),
            file!("steps/02-wait.sh", "smoke-test/steps/02-wait.sh"),
            file!("steps/03-background.sh", "smoke-test/steps/03-background.sh"),
            file!("cleanup.sh", "smoke-test/cleanup.sh"),
            HELPER,
        ],
    ),
    (
        "vm",
        &[
            file!("template.toml", "vm/template.toml"),
            file!("workspace.toml", "vm/workspace.toml"),
            file!("steps/01-check.sh", "vm/steps/01-check.sh"),
            file!("steps/02-start.sh", "vm/steps/02-start.sh"),
            file!("steps/03-wait.sh", "vm/steps/03-wait.sh"),
            file!("steps/04-editor.sh", "vm/steps/04-editor.sh"),
            file!("steps/05-terminal.sh", "vm/steps/05-terminal.sh"),
            file!("cleanup.sh", "vm/cleanup.sh"),
            HELPER,
            VM_HELPER,
        ],
    ),
    (
        "web-project",
        &[
            file!("template.toml", "web-project/template.toml"),
            file!("workspace.toml", "web-project/workspace.toml"),
            file!("steps/01-check.sh", "web-project/steps/01-check.sh"),
            file!("steps/02-git.sh", "web-project/steps/02-git.sh"),
            file!("steps/03-install.sh", "web-project/steps/03-install.sh"),
            file!("steps/04-services.sh", "web-project/steps/04-services.sh"),
            file!("steps/05-editor.sh", "web-project/steps/05-editor.sh"),
            file!("steps/06-dev-server.sh", "web-project/steps/06-dev-server.sh"),
            file!("steps/07-wait.sh", "web-project/steps/07-wait.sh"),
            file!("steps/08-browser.sh", "web-project/steps/08-browser.sh"),
            file!("steps/09-terminal.sh", "web-project/steps/09-terminal.sh"),
            file!("cleanup.sh", "web-project/cleanup.sh"),
            HELPER,
        ],
    ),
];

/// Every built-in template, checked. A broken one is a bug, and a test parses them all.
pub fn all() -> Result<Vec<Template>> {
    BUILTINS.iter().map(|(id, files)| Template::parse(id, files)).collect()
}

/// One template, or `not_found` naming the ones there are.
pub fn find(id: &str) -> Result<Template> {
    match BUILTINS.iter().find(|(t, _)| *t == id) {
        Some((id, files)) => Template::parse(id, files),
        None => {
            let names: Vec<&str> = BUILTINS.iter().map(|(id, _)| *id).collect();
            Err(Error::not_found(format!("no workspace template '{id}' (there are: {})", names.join(", "))))
        }
    }
}
