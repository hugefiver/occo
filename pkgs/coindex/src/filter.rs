use std::fmt;
use std::path::Path;

use serde::Serialize;

const MAX_FILE_SIZE: u64 = 1_048_576;
const MAX_AVG_LINE_LENGTH: f64 = 500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    EmptyFile,
    TooLarge,
    Lockfile,
    Dotfile,
    Minified,
    SkipDir,
    BinaryExt,
    BinaryHeader,
    HighNonAscii,
    LongLines,
    Gitignored,
    GitBinary,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFile => write!(f, "empty"),
            Self::TooLarge => write!(f, "too_large"),
            Self::Lockfile => write!(f, "lockfile"),
            Self::Dotfile => write!(f, "dotfile"),
            Self::Minified => write!(f, "minified"),
            Self::SkipDir => write!(f, "skip_dir"),
            Self::BinaryExt => write!(f, "binary_ext"),
            Self::BinaryHeader => write!(f, "binary_header"),
            Self::HighNonAscii => write!(f, "high_non_ascii"),
            Self::LongLines => write!(f, "long_lines"),
            Self::Gitignored => write!(f, "gitignored"),
            Self::GitBinary => write!(f, "git_binary"),
        }
    }
}

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

/// Returns `None` if the file passes path-based checks, or `Some(reason)` if it should be skipped.
pub fn classify_path(path: &Path, size: u64) -> Option<SkipReason> {
    if size == 0 {
        return Some(SkipReason::EmptyFile);
    }
    if size > MAX_FILE_SIZE {
        return Some(SkipReason::TooLarge);
    }

    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
        return Some(SkipReason::Dotfile);
    };

    if LOCKFILES.contains(&file_name) {
        return Some(SkipReason::Lockfile);
    }

    if file_name.starts_with('.') {
        return Some(SkipReason::Dotfile);
    }

    if file_name.ends_with(".min.js")
        || file_name.ends_with(".min.css")
        || file_name.ends_with(".map")
        || file_name.ends_with(".d.ts")
    {
        return Some(SkipReason::Minified);
    }

    if path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|segment| SKIP_DIRS.contains(&segment))
    {
        return Some(SkipReason::SkipDir);
    }

    if path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|segment| {
            segment.starts_with('.') || segment.is_empty() || segment == "." || segment == ".."
        })
    {
        return Some(SkipReason::Dotfile);
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && BINARY_EXTS.contains(&ext.to_ascii_lowercase().as_str())
    {
        return Some(SkipReason::BinaryExt);
    }

    None
}

pub fn can_ingest(path: &Path, size: u64) -> bool {
    classify_path(path, size).is_none()
}

const HEADER_CHECK_SIZE: usize = 8192;

pub fn has_binary_header(content: &[u8]) -> bool {
    let check_len = content.len().min(HEADER_CHECK_SIZE);
    content[..check_len].contains(&0)
}

pub fn classify_content(content: &[u8]) -> Option<SkipReason> {
    if content.is_empty() {
        return Some(SkipReason::EmptyFile);
    }

    if has_binary_header(content) {
        return Some(SkipReason::BinaryHeader);
    }

    let non_ascii = content.iter().filter(|&&b| b > 0x7F).count();
    let non_ascii_ratio = non_ascii as f64 / content.len() as f64;
    if non_ascii_ratio > 0.5 {
        return Some(SkipReason::HighNonAscii);
    }

    let line_count = content.iter().filter(|&&b| b == b'\n').count().max(1);

    let avg_line_len = content.len() as f64 / line_count as f64;
    if avg_line_len > MAX_AVG_LINE_LENGTH {
        return Some(SkipReason::LongLines);
    }

    None
}

pub fn can_ingest_content(content: &[u8]) -> bool {
    classify_content(content).is_none()
}
