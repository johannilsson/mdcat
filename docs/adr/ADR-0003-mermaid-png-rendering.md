# ADR-0003: Mermaid Rendering via PNG and HiDPI-Aware Sizing

**Date**: 2026-03-09
**Status**: Accepted

## Context

ADR-0002 established the mermaid rendering pipeline as: `mmdc` → SVG → `resvg` rasterization → display. Two problems were discovered in practice:

1. **Text rendering failure**: `resvg` cannot render text in `mmdc`'s SVG output. Mermaid SVGs reference fonts by name (e.g. "trebuchet ms") that `resvg` cannot resolve, even with `load_system_fonts()`. Diagrams rendered as boxes and arrows with no text labels.

2. **Theme mismatch**: The default mermaid theme produces dark text on a white background. Combined with `-b transparent`, this makes text invisible on dark terminals.

Additionally, the initial approach to image sizing — forcing a fixed fraction of terminal width — caused upscaling of narrow diagrams and looked wrong. A correct sizing strategy requires knowledge of the terminal's actual cell pixel dimensions.

## Decision

Replace SVG output with **PNG output** from `mmdc`, use the **dark theme**, generate at **2× scale**, and compute display column count from the terminal's actual cell pixel width queried via **`CSI 16 t`**.

**Pipeline:**
```
Mermaid source (from code fence)
    → write to temp .mmd file
    → mmdc -i input.mmd -o output.png -b transparent -t dark -s 2
    → load PNG as DynamicImage (image crate)
    → query terminal cell pixel width via CSI 16 t
    → natural_cols = png_width_px / cell_px
    → display_cols = min(natural_cols, terminal_width_cols)
    → encode at display_cols via Kitty/iTerm2/blocks protocol
    → clean up temp files
```

## Why PNG instead of SVG

`mmdc` uses Puppeteer/Chromium internally. Chromium has a complete, production-grade font stack; its PNG output faithfully renders all text. The SVG output references fonts by name — `resvg` cannot resolve them reliably, causing text to silently disappear with no error.

Switching to PNG output bypasses `resvg` for mermaid entirely. The loss of SVG scalability is irrelevant since diagrams are displayed at a fixed column count anyway.

## Why `-t dark`

The default mermaid theme uses dark text and light node fills. On a dark terminal with a transparent background (`-b transparent`), the text and fills become invisible. The `dark` theme uses light-colored text and borders designed for dark backgrounds, and works correctly on any terminal with a dark background.

## Why `-s 2` (2× scale)

Terminals on HiDPI/retina displays have physical cell dimensions that are 2× the logical size. The `CSI 16 t` escape sequence reports cell width in **physical pixels**. Running `mmdc` at `-s 2` makes Puppeteer render the PNG at physical pixel resolution, so the formula `natural_cols = png_width / cell_px` maps correctly to terminal columns on retina displays. On 1× displays the image has twice the pixels needed but is still displayed at the correct column count.

## Natural sizing via `CSI 16 t`

Rather than forcing a fixed fraction of terminal width, display each diagram at its natural size and scale down only if wider than the terminal:

```
natural_cols = png_pixel_width / cell_pixel_width
display_cols = min(natural_cols, terminal_width_cols)
```

The terminal's cell pixel width is obtained by sending `CSI 16 t` ("Report Character Cell Size in Pixels") to `/dev/tty` and parsing the response (`ESC [ 6 ; {height} ; {width} t`). This is a standard xterm extension supported by iTerm2, Kitty, Ghostty, and WezTerm.

Fallback chain if `CSI 16 t` is unavailable:
1. `TIOCGWINSZ` pixel fields (`ws_xpixel / ws_col`)
2. Hard-coded default of 10 px/cell

This ensures diagrams narrower than the terminal display at natural size without upscaling, while diagrams wider than the terminal are scaled down to fit.

## Consequences

**Benefits**:
- Text renders correctly — Chromium's font stack handles all mermaid font references
- Dark theme works on dark terminals
- Natural sizing with HiDPI accuracy; narrow diagrams are not artificially stretched
- Graceful degradation if `mmdc` is absent (code block fallback from ADR-0002 retained)

**Trade-offs**:
- PNG output is larger than SVG; slightly more temp file I/O
- `resvg` is no longer used for mermaid (still used for SVG image files)
- `CSI 16 t` requires a TTY; falls back gracefully when unavailable

**Risks**:
- Puppeteer/Chromium can fail in sandboxed or CI environments
- `-s 2` assumes HiDPI; the sizing formula still works on 1× displays but sends a larger PNG than necessary
