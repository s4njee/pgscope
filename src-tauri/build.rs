/// Generates the Tauri context — the parsed `tauri.conf.json`, the bundled
/// frontend assets and the capability/permission set that `generate_context!`
/// expands at compile time. Editing `tauri.conf.json` only takes effect because
/// this reruns.
///
/// # Arguments
/// None.
///
/// # Returns
/// `()` — returns once the context has been generated and the rerun directives
/// emitted; a failure inside `tauri_build::build` panics and fails the build.
fn main() {
    tauri_build::build()
}
