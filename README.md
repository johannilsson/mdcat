# mdcat

Render markdown in the terminal — with syntax highlighting, images, and diagrams.

```bash
mdcat README.md
cat README.md | mdcat
mdcat --pager README.md   # scroll long files
```

## Features

- Syntax-highlighted code blocks
- Images via Kitty, iTerm2, or Unicode block fallback
- Mermaid diagrams (requires `mmdc` from mermaid-cli)
- SVG support
- Built-in pager that preserves terminal graphics

## Install

```bash
cargo install --path .

# With network image support
cargo install --path . --features network-images
```

## Usage

```
Cat for markdown - render formatted markdown in the terminal

Usage: mdcat [OPTIONS] [FILES]... [COMMAND]

Commands:
  render       Render markdown files (default behavior)
  completions  Generate shell completions
  help         Print this message or the help of the given subcommand(s)

Arguments:
  [FILES]...  Files to render (reads from stdin if omitted)

Options:
      --pager
          Enable pager mode (like less)
      --width <WIDTH>
          Terminal width override (defaults to detected width or 80) [env: COLUMNS=]
      --no-images
          Disable inline image rendering
      --no-mermaid
          Disable Mermaid diagram rendering
      --mermaid-binary <MERMAID_BINARY>
          Path to mmdc binary (mermaid-cli) [env: MMDC_PATH=] [default: mmdc]
      --theme <THEME>
          Syntax highlighting theme [default: base16-ocean.dark]
      --image-protocol <PROTOCOL>
          Image rendering protocol override [possible values: auto, kitty, iterm2, sixel,
          blocks]
  -h, --help
          Print help
  -V, --version
          Print version
```


Symlink `mdless → mdcat` to auto-enable pager mode.

## License

MIT
