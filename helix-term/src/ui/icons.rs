use std::path::Path;

use helix_view::graphics::Color;

type Icon = (&'static str, Color);

pub const UNSAVED_DOT: &str = "\u{f444}";

const RUST: Icon = ("\u{e7a8}", Color::Rgb(0xde, 0xa5, 0x84));
const JS: Icon = ("\u{e74e}", Color::Rgb(0xcb, 0xcb, 0x41));
const TS: Icon = ("\u{e628}", Color::Rgb(0x51, 0x9a, 0xba));
const REACT: Icon = ("\u{e7ba}", Color::Rgb(0x51, 0x9a, 0xba));
const PYTHON: Icon = ("\u{e606}", Color::Rgb(0xff, 0xbc, 0x03));
const RUBY: Icon = ("\u{e739}", Color::Rgb(0x70, 0x15, 0x16));
const GO: Icon = ("\u{e627}", Color::Rgb(0x51, 0x9a, 0xba));
const JAVA: Icon = ("\u{e738}", Color::Rgb(0xcc, 0x3e, 0x44));
const C: Icon = ("\u{e61e}", Color::Rgb(0x59, 0x9e, 0xff));
const CPP: Icon = ("\u{e61d}", Color::Rgb(0xf3, 0x4b, 0x7d));
const PHP: Icon = ("\u{e73d}", Color::Rgb(0xa0, 0x74, 0xc4));
const HTML: Icon = ("\u{e736}", Color::Rgb(0xe4, 0x4d, 0x26));
const CSS: Icon = ("\u{e749}", Color::Rgb(0x42, 0xa5, 0xf5));
const JSON: Icon = ("\u{e60b}", Color::Rgb(0xcb, 0xcb, 0x41));
const MARKDOWN: Icon = ("\u{e73e}", Color::Rgb(0x6d, 0x80, 0x86));
const CONFIG: Icon = ("\u{e615}", Color::Rgb(0x6d, 0x80, 0x86));
const LOCK: Icon = ("\u{f023}", Color::Rgb(0x6d, 0x80, 0x86));
const SHELL: Icon = ("\u{f489}", Color::Rgb(0x4d, 0x5a, 0x5e));
const LUA: Icon = ("\u{e620}", Color::Rgb(0x51, 0xa0, 0xcf));
const NIX: Icon = ("\u{f313}", Color::Rgb(0x7e, 0xba, 0xe4));
const VIM: Icon = ("\u{e7c5}", Color::Rgb(0x01, 0x98, 0x33));
const DOCKER: Icon = ("\u{e7b0}", Color::Rgb(0x45, 0x8e, 0xe6));
const GIT: Icon = ("\u{e702}", Color::Rgb(0xf5, 0x4d, 0x27));
const DATABASE: Icon = ("\u{e706}", Color::Rgb(0x6d, 0x80, 0x86));
const IMAGE: Icon = ("\u{f1c5}", Color::Rgb(0xa0, 0x74, 0xc4));
const SVG: Icon = ("\u{f1c5}", Color::Rgb(0xff, 0xb1, 0x3b));
const PDF: Icon = ("\u{f1c1}", Color::Rgb(0xb3, 0x0b, 0x00));
const ARCHIVE: Icon = ("\u{f1c6}", Color::Rgb(0xec, 0xa5, 0x17));
const TEXT: Icon = ("\u{f15c}", Color::Rgb(0x89, 0xe0, 0x51));
const DEFAULT: Icon = ("\u{f15b}", Color::Rgb(0x6d, 0x80, 0x86));

const FOLDER_COLOR: Color = Color::Rgb(0x90, 0xa4, 0xae);
const FOLDER_OPEN: Icon = ("\u{f07c}", FOLDER_COLOR);
const FOLDER_CLOSED: Icon = ("\u{f07b}", FOLDER_COLOR);

pub fn folder_icon(expanded: bool) -> Icon {
    if expanded {
        FOLDER_OPEN
    } else {
        FOLDER_CLOSED
    }
}

pub fn file_icon(path: &Path) -> Icon {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    match name.to_ascii_lowercase().as_str() {
        "cargo.toml" | "cargo.lock" => return RUST,
        "dockerfile" => return DOCKER,
        ".gitignore" | ".gitattributes" | ".gitmodules" => return GIT,
        "flake.nix" | "flake.lock" => return NIX,
        _ => {}
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match ext.as_str() {
        "rs" => RUST,
        "js" | "mjs" | "cjs" => JS,
        "ts" => TS,
        "tsx" | "jsx" => REACT,
        "py" | "pyi" | "pyw" => PYTHON,
        "rb" => RUBY,
        "go" => GO,
        "java" => JAVA,
        "c" | "h" => C,
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => CPP,
        "php" => PHP,
        "html" | "htm" => HTML,
        "css" | "scss" | "sass" => CSS,
        "json" | "jsonc" => JSON,
        "md" | "markdown" => MARKDOWN,
        "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" => CONFIG,
        "lock" => LOCK,
        "sh" | "bash" | "zsh" | "fish" => SHELL,
        "lua" => LUA,
        "nix" => NIX,
        "vim" => VIM,
        "sql" => DATABASE,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => IMAGE,
        "svg" => SVG,
        "pdf" => PDF,
        "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "zst" => ARCHIVE,
        "txt" | "log" => TEXT,
        _ => DEFAULT,
    }
}
