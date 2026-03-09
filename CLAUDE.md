# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build
cargo build --release

# Run
cargo run -- README.md
cargo run -- --pager README.md
cat README.md | cargo run

# Build with network image support
cargo build --features network-images

# Check / lint
cargo check
cargo clippy

# Tests
cargo test
```

## Architecture

`mdcat` is a terminal markdown renderer. The data flow is:

```
files/stdin → input::collect() → [Source] → render::render_all() → ANSI string → pager or stdout
```

**`src/input.rs`** — Collects input sources from file paths or stdin into `Source` structs (name, content, base_dir for resolving relative image paths).

**`src/render/`** — The rendering pipeline:
- `mod.rs` — `Config` struct passed through the whole pipeline; `render_all()` joins multiple sources with separator lines
- `markdown.rs` — Core: walks the `comrak` AST recursively via `render_node()` / `render_inline()`. Dispatches to specialized renderers for code blocks (mermaid or syntax-highlighted), images, links, tables
- `code.rs` — Syntax highlighting via `syntect`
- `images.rs` — Image loading (local/SVG/remote) + `rasterize_svg()` (resvg → tiny-skia → DynamicImage) + `render_dynamic_image()` (viuer). **Note:** viuer prints directly to stdout — it cannot be captured to a string. The function returns `Ok(String::new())` after viuer has already printed.
- `mermaid.rs` — Shells out to `mmdc` (mermaid-cli): writes `.mmd` temp file → runs `mmdc -b transparent` → reads SVG → rasterizes via `images::rasterize_svg()` → renders via viuer. Gracefully degrades to a code block with install hint if `mmdc` is not found.
- `links.rs` — OSC 8 hyperlink escape sequences

**`src/terminal.rs`** — Terminal width/height detection, TTY check, and image protocol detection. Protocol priority: Kitty (`$TERM=xterm-kitty`) → iTerm2/WezTerm (`$TERM_PROGRAM`) → Unicode blocks fallback. Overridable via `--image-protocol`.

**`src/pager.rs`** — Wraps `minus::page_all()`. The embedded pager (rather than spawning `less`) is critical: it preserves terminal graphics escape sequences (Kitty/iTerm2 protocols) that external pagers would strip.

**`src/main.rs`** — CLI parsing via `clap` (derive API). Pager activation logic: `--pager` flag, `--no-pager` flag, invoked-as-`mdless` detection, or auto (TTY + content exceeds terminal height).

## Development guidelines

- For every code change, write unit tests covering the affected logic.
- Run `cargo test` after changes to confirm all tests pass.

## Key design decisions

- **Embedded pager**: `minus` is used instead of spawning `less -R` so that terminal graphics escape sequences survive pagination intact (see ADR-0001).
- **Mermaid**: Shelled out to `mmdc` (Node.js mermaid-cli). The pipeline is: source → temp `.mmd` → `mmdc` → SVG → `resvg` rasterize → `viuer` display (see ADR-0002). Graceful degradation if `mmdc` is absent.
- **Network images**: Behind the `network-images` feature flag (enables `ureq`). Off by default.
- **`mdless` symlink**: Symlinking `mdless → mdcat` auto-enables pager mode, following the `vim`/`view` convention.
- **viuer side-effect**: Image rendering bypasses the string-building pipeline — viuer writes directly to stdout. The ANSI output string and viuer output are interleaved on stdout, which means rendering order matters.
