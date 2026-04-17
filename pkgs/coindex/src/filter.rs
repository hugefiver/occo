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
    "tmp",
    "temp",
    "logs",
    "coverage",
    "cache",
];

const BINARY_EXTS: &[&str] = &[
    // Executables & shared libraries
    "exe", "dll", "so", "dylib", "o", "a", "lib", "obj", "bin", "com", "msi", "apk", "deb", "rpm",
    // Data & databases
    "dat", "db", "sqlite", "sqlite3", "mdb", "accdb", "ldb", "parquet", "arrow", "avro",
    // Images
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg", "webp", "tiff", "tif", "psd", "ai", "eps",
    "heic", "heif", "raw", "cr2", "nef", "dng", "icns",
    // Fonts
    "woff", "woff2", "ttf", "eot", "otf",
    // Audio
    "mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "opus",
    // Video
    "mp4", "avi", "mov", "mkv", "wmv", "flv", "webm", "m4v", "mpg", "mpeg",
    // Archives & compressed
    "zip", "tar", "gz", "bz2", "7z", "rar", "xz", "zst", "lz4", "lzma", "cab", "dmg", "iso",
    // Java / JVM
    "jar", "war", "ear", "class",
    // Python bytecode
    "pyc", "pyo", "pyd",
    // .NET
    "nupkg",
    // WebAssembly
    "wasm",
    // Documents (binary formats)
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp",
    // Misc binary
    "swf", "fla", "blend", "fbx", "glb", "gltf", "3ds", "dwg", "dxf",
    "DS_Store", "thumbs.db",
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
                || segment.is_empty()
                || segment == "."
                || segment == ".."
        })
    {
        return false;
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && BINARY_EXTS.contains(&ext.to_ascii_lowercase().as_str())
    {
        return false;
    }

    true
}

const HEADER_CHECK_SIZE: usize = 8192;

pub fn has_binary_header(content: &[u8]) -> bool {
    let check_len = content.len().min(HEADER_CHECK_SIZE);
    content[..check_len].contains(&0)
}

pub fn can_ingest_content(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }

    if has_binary_header(content) {
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
