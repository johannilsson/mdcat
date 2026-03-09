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
mdcat [OPTIONS] [FILE]...

Options:
  --pager       Force pager mode
  --no-pager    Disable pager
  --image-protocol <PROTOCOL>  kitty | iterm2 | blocks
```

Symlink `mdless → mdcat` to auto-enable pager mode.

## License

MIT
