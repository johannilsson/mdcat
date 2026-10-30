# Release notes

mdcat renders **bold**, *italic*, `inline code`, ~~strikethrough~~ and
[clickable links](https://github.com/johannilsson/mdcat) right in your terminal.

> Block quotes are wrapped to fit the terminal width, and footnotes[^1] are
> collected at the end of the document.

```rust
fn main() {
    let langs = ["rust", "python", "go"];
    for (i, lang) in langs.iter().enumerate() {
        println!("{i}: {lang}");
    }
}
```

| Feature        | Status | Notes                    |
|----------------|:------:|--------------------------|
| Code blocks    |   ✓    | 100+ languages (syntect) |
| Tables         |   ✓    | Alignment and wrapping   |
| Mermaid        |   ✓    | Requires `mmdc`          |

- [x] Task lists
- [x] Nested lists
  1. Ordered, with custom start numbers
  2. And correct numbering
- [ ] World domination

[^1]: Like this one.
