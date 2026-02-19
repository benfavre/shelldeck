pub mod buffer;
pub mod file_browser;
pub mod highlighter;
pub mod input;
pub mod view;

use std::path::Path;

/// Classification of a file before opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Pdf,
    Binary,
    Text,
}

impl FileKind {
    /// Classify by file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // Image formats supported by GPUI
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "tif" | "tiff"
            | "avif" | "tga" | "qoi" => Self::Image,
            // PDF
            "pdf" => Self::Pdf,
            // Known binary formats
            "exe" | "dll" | "so" | "dylib" | "o" | "a" | "lib" | "bin" | "class" | "pyc"
            | "pyd" | "wasm" | "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar"
            | "iso" | "dmg" | "deb" | "rpm" | "mp3" | "mp4" | "mkv" | "avi" | "mov" | "flac"
            | "wav" | "ogg" | "m4a" | "aac" | "ttf" | "otf" | "woff" | "woff2" | "eot"
            | "sqlite" | "db" | "dat" => Self::Binary,
            // Everything else: attempt as text
            _ => Self::Text,
        }
    }

    /// Classify by full filename, handling special cases then falling back to extension.
    pub fn from_filename(name: &str) -> Self {
        let basename = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name);

        if let Some(ext) = Path::new(basename).extension().and_then(|e| e.to_str()) {
            Self::from_extension(ext)
        } else {
            // No extension — treat as text (Makefile, Dockerfile, etc.)
            Self::Text
        }
    }
}

/// Supported editor languages with tree-sitter grammar support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Bash,
    Json,
    Toml,
    Yaml,
    Html,
    Css,
    Sql,
    PlainText,
}

impl EditorLanguage {
    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Self::Rust,
            "js" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "tsx" | "jsx" => Self::TypeScript,
            "py" | "pyw" | "pyi" => Self::Python,
            "sh" | "bash" | "zsh" | "fish" => Self::Bash,
            "json" | "jsonc" | "json5" => Self::Json,
            "toml" => Self::Toml,
            "yml" | "yaml" => Self::Yaml,
            "html" | "htm" | "xhtml" => Self::Html,
            "css" | "scss" | "less" => Self::Css,
            "sql" => Self::Sql,
            _ => Self::PlainText,
        }
    }

    /// Detect language from a full filename (handles special cases like Makefile, Dockerfile).
    pub fn from_filename(name: &str) -> Self {
        let basename = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name);

        match basename {
            "Makefile" | "makefile" | "GNUmakefile" => Self::Bash,
            "Dockerfile" => Self::Bash,
            ".bashrc" | ".bash_profile" | ".zshrc" | ".profile" => Self::Bash,
            "Cargo.toml" | "pyproject.toml" => Self::Toml,
            "tsconfig.json" | "package.json" => Self::Json,
            _ => {
                if let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) {
                    Self::from_extension(ext)
                } else {
                    Self::PlainText
                }
            }
        }
    }

    pub fn comment_prefix(&self) -> Option<&'static str> {
        match self {
            Self::Rust | Self::JavaScript | Self::TypeScript | Self::Css | Self::Sql => {
                Some("// ")
            }
            Self::Python | Self::Bash | Self::Toml | Self::Yaml => Some("# "),
            Self::Html | Self::Json | Self::PlainText => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Python => "Python",
            Self::Bash => "Shell",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Yaml => "YAML",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Sql => "SQL",
            Self::PlainText => "Plain Text",
        }
    }
}
