# Code review: optimizations and improvements

Date: 2026-07-06
Scope: full review of the `mdcat` binary crate (`src/`) and the `kitty-pager` workspace crate.
Baseline: `cargo test --workspace` passes (18 tests), `cargo clippy --all-targets` is clean.

Items marked **[verified]** were reproduced against a debug build during the review; the rest
come from code reading.

---

## 1. Confirmed rendering bugs

### 1.1 Multi-file separator is ~2× the terminal width — [verified]

`src/render/mod.rs:75-76` computes the dash count as `width - header.len() - 2`, then prints
that run of dashes **twice** (left and right of the header):

```rust
let dashes = "─".repeat((width.saturating_sub(header.len() + 2)).max(4));
output.push_str(&format!("\x1b[2m{dashes}{header}{dashes}\x1b[0m\n\n"));
```

At `--width 40` with two files, the separator line renders 198 visible characters. Fix: split
the remaining width in halves like `render_stacked_table` already does
(`src/render/markdown.rs:409-411`), and measure the header with a display-width function
rather than `str::len()` (bytes).

Related inconsistency: the separator names only files after the first (`i > 0`), so with
multiple inputs the first file is never labeled. Either print a header for every file when
`sources.len() > 1`, or none.

### 1.2 Hard line breaks are collapsed — [verified]

`NodeValue::LineBreak` pushes `'\n'` (`src/render/markdown.rs:228-231`), but the paragraph is
then passed through `word_wrap`, which iterates `text.split_whitespace()`
(`src/render/markdown.rs:451`) and treats that `\n` as ordinary whitespace. A markdown hard
break (trailing two spaces or `\`) is silently joined into one line:

```
line one␣␣
line two        →   "line one line two"
```

Fix: wrap each `\n`-separated segment independently inside `word_wrap` (or use a sentinel
that survives wrapping).

### 1.3 Ordered lists ignore their start number — [verified]

`src/render/markdown.rs:128` initializes `counter = 1` unconditionally. A list beginning
`5. five` renders as `1. five`. comrak provides the start via `NodeList::start`; seed the
counter from it.

### 1.4 List item first line is misaligned — [verified]

`render_list_item` (`src/render/markdown.rs:317`) emits `{prefix}• ` and then the child
paragraph emits its own `indent`-space prefix on the same line, so the first line gets
bullet + space + 2 more spaces (`1.   five`), while wrapped continuation lines indent by only
`indent`. The paragraph renderer should skip its prefix on the first line when it directly
follows a bullet (e.g. pass a "first-line already prefixed" flag, or trim the first prefix in
`render_list_item`).

---

## 2. Correctness risks (from code reading)

### 2.1 `visible_len` mishandles OSC 8 hyperlinks

`src/render/markdown.rs:473-486` assumes every escape sequence ends with `m` (true only for
SGR). OSC 8 links (`\x1b]8;;https://example.com\x1b\\`) end with `ESC \` — the scanner exits
"escape mode" at the first literal `m` it meets, which for most URLs is a character *inside*
the URL (`example.com`). Every link makes wrap-width accounting wrong for the rest of the
word. Fix: recognize CSI (`ESC [ … final-byte @–~`), OSC (`ESC ] … BEL` or `ESC \`), and APC
(`ESC _ … ESC \`) terminators. This function is the foundation of all wrapping, so it is
worth a thorough unit-test suite.

### 2.2 Byte length used where display width is needed

Several places measure text with `str::len()` (bytes) but pad/draw in terminal columns:

- table column widths: `cell.len()` at `src/render/markdown.rs:364` (and `{:<width$}`
  pads by *chars*, a third unit — non-ASCII cells shift borders; CJK/emoji are worse
  since even `chars().count()` ≠ columns)
- heading underline length: `text.len()` at `src/render/markdown.rs:57,63`
- stacked-table label width: `h.len()` at `src/render/markdown.rs:405`
- file separator header (see 1.1)

Recommendation: add the `unicode-width` crate and one helper (`display_width(&str)`), use it
everywhere, and delete the per-site ad-hoc measurements.

### 2.3 Inline code spans break under word wrap

Inline code renders as `\x1b[48;5;236m\x1b[96m {text} \x1b[0m`
(`src/render/markdown.rs:263`). `word_wrap`'s `split_whitespace` then re-splits multi-word
code spans, dropping the padding spaces and allowing a wrap point mid-span, so the background
highlight fragments or bleeds. Wrapping on styled text with a byte-oriented splitter is
fragile in general; consider wrapping *before* styling (wrap plain text runs, then apply
styles), or make the wrapper escape-sequence-aware and treat a styled span as atomic.

### 2.4 A file named `render` or `completions` cannot be rendered

`Cli` mixes a positional `files` list with subcommands (`src/main.rs:19-66`). `mdcat render`
parses as the `Render` subcommand with no files and blocks reading stdin. The `Render`
subcommand adds nothing over the default positional path — removing it (and implementing or
removing `completions`, see 4.4) eliminates the ambiguity.

### 2.5 Panic on non-UTF-8 `argv[0]`

`std::env::args()` (`src/main.rs:72`) panics if any argument is not valid Unicode. The
mdless detection only needs `argv[0]`; use `std::env::args_os().next()` and compare the
`OsStr` file name.

### 2.6 Footnote references vanish

`options.extension.footnotes = true` (`src/render/markdown.rs:19`) makes comrak parse `[^1]`
into `FootnoteReference` nodes, which fall into the `_` arm of `render_inline` and render
nothing (they have no children). `FootnoteDefinition` is likewise skipped
(`src/render/markdown.rs:168-171`). Net effect: enabling the extension makes footnote markers
*disappear from the output* — strictly worse than leaving the extension off. Either render
`[n]` superscripts plus definitions at the end, or disable the extension until implemented.

### 2.7 HTML blocks and inline HTML are dropped silently

`HtmlBlock` is skipped (`src/render/markdown.rs:118-121`) and `HtmlInline` falls through the
`_` arm with no children. Content like `<br>`, `<img src=…>`, `<details>` bodies, or a
`<table>` disappears without a trace. Minimum improvement: emit the literal dimmed, or at
least handle the common cases (`<br>` → newline).

### 2.8 `mdcat - -` reads stdin twice

`src/input.rs:29-38`: a second `-` re-reads stdin after the first drained it, yielding an
empty source. Read stdin once and reuse it (this also matters for `mdcat a.md - b.md -`).

### 2.9 Remote SVG images fail to decode

`load_remote_image` (`src/render/images.rs:195-208`) always calls
`image::load_from_memory`, which cannot decode SVG — but `load_local_image` special-cases the
`.svg` extension. A remote `.svg` URL therefore errors. Sniff the payload (or URL extension /
Content-Type) and route to `rasterize_svg` like the local path.

### 2.10 `rasterize_svg` truncates fractional sizes

`size.width() as u32` (`src/render/images.rs:179-180`) floors the float; an SVG with a
declared size < 1.0 gives 0 and `Pixmap::new` fails, and e.g. 99.7px loses a pixel. Use
`.ceil() as u32` and clamp to at least 1.

### 2.11 Mermaid: `which` shell-out is unnecessary and non-portable

`which_mmdc` (`src/render/mermaid.rs:61-69`) spawns `which`, which doesn't exist on Windows
and is racy (TOCTOU vs. the real spawn). Just run `mmdc` and map
`std::io::ErrorKind::NotFound` to the friendly "install mmdc" message. Also:
`input_file.path().to_str().unwrap()` (`src/render/mermaid.rs:36-37`) panics on non-UTF-8 temp
paths — pass the `Path` directly via `.arg(input_file.path())`; the variable `status` at
`src/render/mermaid.rs:34` actually holds an `Output` (naming nit); and the `-s 3` scale
factor is hardcoded — consider deriving it from the cell pixel size or exposing a flag.

### 2.12 `--image-protocol sixel` silently renders blocks

`sixel` is an accepted CLI value (`src/main.rs:51`) but `render_dynamic_image` maps
`Sixel → blocks_encode` (`src/render/images.rs:40`). Either implement sixel or drop it from
the `value_parser` so users aren't misled.

### 2.13 Inconsistent cell-size fallbacks

`src/terminal.rs:31` falls back to a cell width of 10 while
`kitty-pager/src/terminal.rs:10` falls back to 8 (and heights 20 vs 16). Both feed the same
layout math on the Kitty-pager path (`src/main.rs:127-132` uses the mdcat versions), so a
terminal that answers neither query gets mixed geometry. Pick one source of truth (see 4.1).

---

## 3. Performance optimizations

### 3.1 Cell-size TTY query runs per image, 200 ms timeout each — biggest win

`query_cell_size_csi16t` (`src/terminal.rs:50-112`) opens `/dev/tty`, flips raw mode, and
waits up to 200 ms for a reply. It is invoked:

- once per image via `render_image` → `cell_pixel_width()` (`src/render/images.rs:11`)
- once per mermaid diagram (`src/render/mermaid.rs:54`)
- twice more on the Kitty-pager path (`src/main.rs:130-131` — width and height each issue a
  *separate* full query)

On any terminal that doesn't answer CSI 16t, a document with N images stalls ~200 ms × N.
Fix: query once, cache both dimensions in a `std::sync::OnceLock<(u32, u32)>`, and have
`cell_pixel_width`/`cell_pixel_height` read from it. This is a small, isolated change with a
large latency payoff.

### 3.2 Resolve the image protocol once

`detect_image_protocol` re-reads environment variables for every image
(`src/render/images.rs:37`) and again in `main` (`src/main.rs:109`). Store the resolved
`ImageProtocol` enum in `Config` (replacing the raw `Option<String>`, ideally as a clap
`ValueEnum`) and thread it through. Cheap, and it removes a stringly-typed field.

### 3.3 `build_kitty_document` deep-copies every image

`parse_sentinel` (`src/render/mod.rs:143`) clones `rgba_data`. A 3000×1500 mermaid PNG at
RGBA8 is ~18 MB, held twice for the lifetime of the pager. Wrap the pixel buffer in
`Rc<Vec<u8>>`/`Arc<[u8]>`, or drain images out of the store by value instead of cloning.

### 3.4 Cache mermaid renders

Every invocation shells out to `mmdc`, which boots headless Chromium — seconds per diagram,
every time the same document is viewed. Cache the output PNG keyed by a hash of
(source, theme, scale, background) under `$XDG_CACHE_HOME/mdcat/` with a size cap. This turns
repeat views from seconds into milliseconds and needs no new heavyweight dependencies
(a small hash of the input suffices).

### 3.5 `blocks_encode` emits ~40 bytes per pixel

`src/render/images.rs:117-133` writes a full `38;2;…m` + `48;2;…m` pair for every cell. Track
the previous fg/bg and emit a new SGR only when the color changes — flat-color regions
(most diagrams, screenshots, logos) shrink dramatically, which also speeds up the terminal's
parser. Also consider `image::imageops::FilterType::Triangle` instead of `Lanczos3` here:
at half-block resolution the quality difference is invisible and it is several times faster.

### 3.6 Syntect setup

`src/render/code.rs:6-9` loads `SyntaxSet`/`ThemeSet` in `thread_local!`. The program is
single-threaded, so this works, but a `OnceLock` static states the intent (load once per
process) and avoids re-loading if rendering is ever parallelized. Separately, the
`default-fancy` feature selects the pure-Rust `fancy-regex` engine, which is markedly slower
than the `default-onig` (Oniguruma) path — if the C dependency is acceptable, switching
speeds up highlighting of large code blocks; if not, this is a documented trade-off worth a
comment in `Cargo.toml`.

### 3.7 Minor allocation churn

- `kitty_encode` / `kitty_transmit` collect chunk slices into a `Vec` just to know the count
  (`src/render/images.rs:77`, `kitty-pager/src/renderer.rs:238`) — compute
  `total = b64.len().div_ceil(4096)` and iterate directly.
- Rendering builds strings via nested `format!` + `push_str`; `write!(output, …)` with
  `std::fmt::Write` avoids the intermediate `String` per line. Mostly style, but it is the
  hot loop for big documents.
- `layout` (`kitty-pager/src/renderer.rs:41-46`) copies every text line into a new `String`,
  roughly doubling document memory in the pager; storing `(item_idx, byte_range)` and slicing
  at render time avoids it.
- `input.rs:15` allocates a `Vec` just to compare: `files == vec![PathBuf::from("-")]` →
  `matches!(files.as_slice(), [p] if p.as_os_str() == "-")`.

---

## 4. Code structure and duplication

### 4.1 Duplicate CSI-16t implementations

`src/terminal.rs:50-112` and `kitty-pager/src/terminal.rs:14-76` are near-identical raw-mode
`/dev/tty` query routines (with different fallbacks, see 2.13). mdcat already depends on
`kitty-pager`; delete the mdcat copy and call
`kitty_pager::terminal::query_cell_pixel_size()` (adding the caching from 3.1 there).

### 4.2 Duplicate Kitty transmission encoders

`kitty_encode` (`src/render/images.rs:65-93`) and `kitty_transmit` + `kitty_place`
(`kitty-pager/src/renderer.rs:208-256`) both implement chunked base64 APC transmission.
Export one encoder from `kitty-pager` (parameterized on `a=T` vs `a=t,i=…`) and reuse it.

### 4.3 `Config` carries unresolved state

`Config.image_protocol: Option<String>` and `theme: String` are re-parsed/looked up at every
use site. Resolving protocol (3.2) and theme (a `&'static Theme` or index) at startup makes
invalid values fail fast at CLI parse time instead of silently falling back per code block.

### 4.4 Dead/stub code

- `completions` subcommand prints "not yet implemented" (`src/main.rs:165-170`). Wiring up
  `clap_complete` is ~10 lines; otherwise remove the subcommand (also fixes 2.4).
- `Commands::Render` is redundant with the positional-files default path.
- `KittyImageStore` could `impl Default` (idiomatic; clippy pedantic flags it).

### 4.5 Documentation drift

`CLAUDE.md` and the architecture notes describe image rendering via **viuer** ("viuer prints
directly to stdout — it cannot be captured to a string"), but the code no longer uses viuer
at all — `images.rs` builds escape-sequence strings and returns them through the normal
pipeline, and `Cargo.toml` has no viuer dependency. The "rendering order matters because of
stdout interleaving" caveat is obsolete. Update `CLAUDE.md` (and `docs/adr` if applicable) so
future changes aren't designed around a constraint that no longer exists.

---

## 5. Robustness and security notes

- **Network fetch limits** (`src/render/images.rs:196-204`): the 50 MB read cap is good, but
  no request timeout is configured on the `ureq` agent — a stalled server hangs rendering
  indefinitely. Configure connect/overall timeouts explicitly.
- **Image decode limits**: `image::open` / `load_from_memory` are fed untrusted files/bytes
  with default (unbounded) dimension limits; a crafted small file declaring huge dimensions
  can exhaust memory. Set `image::io::Reader` limits (e.g. via `Limits`) before decoding.
- **Terminal state restoration**: both CSI-16t query functions restore termios via a plain
  call after the closure — a panic between `tcsetattr` calls leaves the TTY raw. A small
  RAII guard makes it panic-safe (same applies to `run_pager`'s raw-mode/alt-screen teardown
  in `kitty-pager/src/input.rs:25-36`).

---

## 6. Pager UX gaps (kitty-pager)

- No Space (page down), `u`/`d` without Ctrl, or mouse-wheel scrolling — Space at minimum is
  muscle memory from `less`. Mouse support is one `crossterm` event match arm
  (`kitty-pager/src/input.rs:63-94`).
- `Event::Resize` re-runs `layout` but the document text was word-wrapped at the original
  width upstream, so shrinking the window truncates lines (`\x1b[?7l` disables wrap). A note
  in the docs, or re-rendering on resize, would address it; the current silent behavior is
  the worst of both.
- Scroll floor: `max_top = entries.len() - 1` lets the user scroll until only the last line
  is visible; `less` stops when the last line reaches the bottom. Consider
  `entries.len().saturating_sub(content_rows)`.
- The status bar shows only a percentage; adding the file name (already in `Source.name`)
  and line position would be cheap.

---

## 7. Test coverage

Existing tests cover frontmatter wrapping, sentinel parsing, and the kitty renderer math well.
Missing, and worth adding alongside the fixes above (per the repo's own guideline of tests
with every change):

- `word_wrap` / `visible_len`: OSC 8 links, hard breaks, ANSI-styled words, non-ASCII (§1.2, §2.1)
- table rendering: Unicode cells, overflow → stacked mode threshold (§2.2)
- ordered list start numbers and nested-list indentation (§1.3, §1.4)
- multi-file separator width (§1.1)
- `input::collect`: stdin `-` handling, missing-file error (§2.8)
- `detect_image_protocol` env matrix (needs env isolation — pass an env lookup function in,
  which also removes global-state reads from library code)
- `blocks_encode` output shape (row count, reset codes)

---

## 8. Positive observations

- The Kitty pager design (transmit-once `a=t`, cheap `a=p` re-placement, source-rect cropping
  for partial scroll) is sound and unusually well-commented, with strong regression tests for
  the scaling/cropping math.
- Graceful degradation paths (mermaid → code block with install hint, images → alt text,
  minus-pager path disabling graphics that would be stripped) are consistently thought
  through.
- The release profile (`lto`, `codegen-units = 1`, `strip`) and the minimal `image` feature
  set keep the binary lean.
- `checked_div` on the scroll percentage and the `\x00`-sentinel document splitting are
  careful touches.

## Suggested priority order

1. Cache the cell-size query (§3.1) — one-line-ish change, removes up-to-200 ms-per-image stalls.
2. Separator width fix (§1.1) and hard-break fix (§1.2) — user-visible output corruption.
3. `visible_len` escape-sequence handling + `unicode-width` adoption (§2.1, §2.2) — fixes a
   whole class of wrapping/alignment bugs.
4. Ordered-list start + list indent (§1.3, §1.4).
5. Mermaid cache (§3.4) — biggest perceived-speed win for repeat use.
6. Deduplicate terminal/Kitty-encoder code into `kitty-pager` (§4.1, §4.2).
7. CLAUDE.md viuer drift (§4.5) — cheap, prevents future misdesign.
