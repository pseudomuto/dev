// `include_dir!` in src/new.rs embeds share/templates, but cargo only learns
// about the files that existed at the last build. Without this, a new file or
// template doesn't make it into the binary until something else triggers a
// rebuild. A directory here means "anything under it".
fn main() {
    println!("cargo::rerun-if-changed=share/templates");
}
