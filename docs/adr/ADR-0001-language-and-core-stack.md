# ADR-0001: Language and Core Stack

**Date**: 2026-03-08
**Status**: Accepted

## Context

We are building `mdcat`, a terminal tool that:
- Renders formatted markdown in the terminal (like `cat` but for markdown)
- Acts as a pager for long content (like `less`)
- Renders inline images using terminal graphics protocols (iTerm2, Kitty, Sixel)
- Renders Mermaid diagrams as actual diagrams

The tool needs to feel like a Unix utility: fast startup, single binary, no runtime surprises. Inline image rendering requires emitting specific terminal escape sequences that must survive the pager pipeline intact.

## Decision

Build `mdcat` in **Rust** with the following core dependencies:

| Crate | Purpose |
|---|---|
| `comrak` | Markdown parsing (full GFM AST) |
| `viuer` | Inline image rendering (iTerm2, Kitty, Sixel, block fallback) |
| `syntect` | Syntax highlighting for code blocks |
| `resvg` | SVG rasterization (for Mermaid output) |
| `minus` | Embedded async pager |
| `clap` | CLI argument parsing |

Mermaid rendering shells out to `mmdc` (mermaid-cli), with graceful degradation to a styled code block if `mmdc` is not installed.

The binary is named `mdcat`; a symlink `mdless -> mdcat` activates pager mode automatically (following the `vim`/`view` convention).

## Alternatives Considered

**Python + uv**
- Pros: Fast iteration, readable code, `rich` library for terminal rendering
- Cons: No clean path for inline terminal image rendering; would need to shell out to `chafa` or `viu` for images; slow startup (~100ms) for a `cat` replacement; distribution as a single binary is fragile

**Go**
- Pros: Fast compile, single binary, good ecosystem (Glow/Glamour exist)
- Cons: Thin ecosystem for terminal graphics protocols; Glow itself does not support images or Mermaid; would be rebuilding Glow with the same fundamental limitations

**Node.js**
- Pros: Native Mermaid rendering without shelling out
- Cons: ~100-300ms startup overhead unacceptable for a `cat` replacement; large distribution bundles

## Rationale

The inline image rendering requirement is the deciding factor. `viuer` provides a battle-tested implementation of all three major terminal graphics protocols (iTerm2 OSC 1337, Kitty chunked transfer, Sixel) with automatic terminal detection - roughly 2000 lines of correctly-tested protocol code available for free.

The embedded pager (`minus`) is critical: spawning `less -R` would drop terminal graphics escape sequences. An embedded pager lets the fully-rendered ANSI output (including image sequences) pass through intact.

Rust's ~5ms startup time is appropriate for a `cat` replacement. The binary is fully self-contained - `resvg` is pure Rust with no C FFI or system library dependencies, which keeps the build and distribution simple on macOS.

The existing (now archived) `swsnr/mdcat` project validated this exact stack. Building a maintained successor is the clear opportunity.

## Consequences

**Benefits**:
- Single self-contained binary, trivial to distribute and install
- ~5ms startup time, appropriate for interactive use
- `viuer` handles all terminal graphics protocol complexity
- `resvg` keeps the build fully self-contained (no Homebrew C library deps)
- Embedded pager allows inline images to survive pagination

**Trade-offs**:
- Rust has a steeper learning curve and longer compile times than Python or Go
- Mermaid rendering requires `mmdc` (Node.js) as an optional runtime dependency

**Risks**:
- `mmdc` / Puppeteer can be flaky on some systems; mitigation is graceful degradation
- Terminal graphics protocol support varies; `viuer`'s auto-detection may misidentify some terminals - need to expose `--image-protocol` override flag
