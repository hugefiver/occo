use std::path::Path;

const MAX_FILE_SIZE: u64 = 1_048_576;
const MAX_AVG_LINE_LENGTH: f64 = 500.0;

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "target",
    "dist",
    "build",
    ".git",
    "__pycache__",
    ".venv",
    "venv",
];

const BINARY_EXTS: &[&str] = &[
    "exe", "dll", "so", "dylib", "o", "a", "lib", "obj", "bin", "dat", "db", "sqlite", "png",
    "jpg", "jpeg", "gif", "bmp", "ico", "svg", "woff", "woff2", "ttf", "eot", "mp3", "mp4", "avi",
    "mov", "zip", "tar", "gz", "bz2", "7z", "rar", "jar", "war", "class", "pyc", "pyo", "wasm",
];

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
    "bun.lock",
    "bun.lockb",
];

pub fn can_ingest(path: &Path, size: u64) -> bool {
    if size == 0 || size > MAX_FILE_SIZE {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };

    if LOCKFILES.contains(&file_name) {
        return false;
    }

    if file_name.starts_with('.') {
        return false;
    }

    if file_name.ends_with(".min.js")
        || file_name.ends_with(".min.css")
        || file_name.ends_with(".map")
        || file_name.ends_with(".d.ts")
    {
        return false;
    }

    if path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|segment| {
            segment.starts_with('.')
                || SKIP_DIRS.contains(&segment)
                || segment == ""
                || segment == "."
                || segment == ".."
        })
    {
        return false;
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if BINARY_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
            return false;
        }
    }

    true
}

pub fn can_ingest_content(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }

    let non_ascii = content.iter().filter(|&&b| b > 0x7F).count();
    let non_ascii_ratio = non_ascii as f64 / content.len() as f64;
    if non_ascii_ratio > 0.5 {
        return false;
    }

    let line_count = content.iter().filter(|&&b| b == b'\n').count().max(1);

    let avg_line_len = content.len() as f64 / line_count as f64;
    if avg_line_len > MAX_AVG_LINE_LENGTH {
        return false;
    }

    true
}
