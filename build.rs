fn main() {
    // ── exe 图标资源 ──────────────────────────────────────────
    // 把 assets/icon.ico 嵌入两个 exe（Explorer/任务栏/Alt-Tab 图标）。
    // 文件不存在时跳过嵌入并警告 — 构建不因此失败。
    // Icon resource for both exes. Skipped with a warning when the
    // .ico is missing — the build must not fail without it.
    let ico = std::path::Path::new("assets/icon.ico");
    if ico.exists() {
        embed_resource::compile("assets/icon.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed exe icon resource");
        println!("cargo:rerun-if-changed=assets/icon.ico");
        println!("cargo:rerun-if-changed=assets/icon.rc");
    } else {
        println!("cargo:warning=assets/icon.ico not found — exe will have no custom icon");
    }
}
