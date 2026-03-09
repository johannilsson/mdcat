# ADR-0002: Mermaid Diagram Rendering

**Date**: 2026-03-08
**Status**: Superseded by ADR-0003

## Context

`mdcat` needs to render Mermaid diagrams (` ```mermaid ` code fences) as actual visual diagrams in the terminal, not as raw source code. Mermaid is a JavaScript-based diagramming language with no native Rust implementation. Any solution must integrate with the image rendering pipeline established in ADR-0001 (viuer + resvg).

## Decision

Shell out to `mmdc` (mermaid-cli) to convert Mermaid source to SVG, then rasterize the SVG with `resvg` and display the resulting image via `viuer`.

**Pipeline:**
```
Mermaid source (from code fence)
    → write to temp file
    → mmdc -i input.mmd -o output.svg -b transparent
    → read SVG bytes
    → resvg: rasterize to pixels
    → viuer: display inline in terminal
    → clean up temp files
```

**Graceful degradation:** If `mmdc` is not found on `PATH`, display the Mermaid source as a syntax-highlighted code block with a notice pointing the user to install `mmdc`.

Expose a `--mermaid-binary` flag to allow specifying a custom path to `mmdc`.

## Alternatives Considered

**Bundle a Node.js runtime (Deno compile or neon)**
- Pros: Zero external runtime dependency for Mermaid; fully self-contained binary
- Cons: Adds 30-50MB to binary size; unacceptable for a `cat` replacement; significant build complexity

**mermaid-isomorphic (isomorphic JS library)**
- Pros: More portable API than raw `mmdc`
- Cons: Still requires Playwright/Puppeteer internally; no meaningful advantage over `mmdc` subprocess, just a different entry point

**Pure-Rust Mermaid parser**
- Pros: No external dependency at all
- Cons: Does not exist; Mermaid's spec is large and actively evolving; building and maintaining a parser is out of scope

**Display as styled code block always (no rendering)**
- Pros: Zero dependency, always works
- Cons: Defeats the purpose; Mermaid diagrams as raw text are often unreadable

**Pre-render at document-write time (CI/build step)**
- Pros: No runtime dependency
- Cons: Not applicable - `mdcat` renders arbitrary markdown at runtime, not build time

## Rationale

`mmdc` is the official Mermaid CLI, widely installed in developer environments (`npm install -g @mermaid-js/mermaid-cli`). Shelling out to it keeps the Rust binary small and delegates diagram fidelity to the canonical implementation. The SVG output slots cleanly into the existing `resvg` → `viuer` pipeline already needed for SVG images.

Graceful degradation ensures `mdcat` remains useful even without `mmdc` - the raw Mermaid source is still shown, just not rendered as a diagram.

## Consequences

**Benefits**:
- Full-fidelity Mermaid rendering via the official implementation
- SVG intermediate format integrates with the existing resvg pipeline
- Transparent background (`-b transparent`) ensures diagrams look correct on any terminal background color
- Graceful degradation means `mmdc` is optional, not required

**Trade-offs**:
- `mmdc` (Node.js + Puppeteer) is a heavy optional dependency
- Subprocess invocation adds latency per diagram (~500ms-2s depending on system)
- Temp file lifecycle must be managed carefully to avoid leaks on error paths

**Risks**:
- `mmdc`/Puppeteer can fail silently or produce empty SVG on some systems; need to validate SVG output before rasterizing
- Puppeteer's Chromium dependency can cause issues in sandboxed environments; document this limitation
- Temp files in `/tmp` could collide in theory; use `tempfile` crate for safe unique paths
