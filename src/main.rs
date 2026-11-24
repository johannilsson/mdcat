use anyhow::Result;
use clap::{Parser, Subcommand};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

mod input;
mod pager;
mod render;
mod terminal;

#[derive(Parser)]
#[command(
    name = "mdcat",
    about = "Cat for markdown - render formatted markdown in the terminal",
    version,
    after_help = "When invoked as 'mdless', pager mode is enabled automatically.\n\nExamples:\n  mdcat README.md\n  mdcat --pager README.md\n  cat README.md | mdcat"
)]
struct Cli {
    /// Files to render (reads from stdin if omitted)
    files: Vec<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,

    /// Enable pager mode (like less)
    #[arg(long)]
    pager: bool,

    /// Terminal width override (defaults to $COLUMNS, then detected width, then 80)
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    width: Option<u16>,

    /// Disable inline image rendering
    #[arg(long)]
    no_images: bool,

    /// Disable Mermaid diagram rendering
    #[arg(long)]
    no_mermaid: bool,

    /// Path to mmdc binary (mermaid-cli)
    #[arg(long, default_value = "mmdc", env = "MMDC_PATH")]
    mermaid_binary: String,

    /// Syntax highlighting theme
    #[arg(long, default_value = "base16-ocean.dark")]
    theme: String,

    /// Image rendering protocol override
    #[arg(long, value_name = "PROTOCOL", value_parser = ["auto", "kitty", "iterm2", "sixel", "blocks"])]
    image_protocol: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Render markdown files (default behavior)
    Render {
        files: Vec<PathBuf>,
    },
    /// Generate shell completions
    Completions {
        #[arg(value_parser = ["bash", "zsh", "fish", "powershell"])]
        shell: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Detect if invoked as "mdless" -> enable pager mode
    let invoked_as_mdless = std::env::args()
        .next()
        .and_then(|a| PathBuf::from(a).file_name().map(|n| n.to_string_lossy().into_owned()))
        .map(|name| name == "mdless")
        .unwrap_or(false);

    let config = render::Config {
        width: resolve_width(
            cli.width,
            std::env::var("COLUMNS").ok().as_deref(),
            terminal::width,
        ),
        images: !cli.no_images,
        mermaid: !cli.no_mermaid,
        mermaid_binary: cli.mermaid_binary.clone(),
        theme: cli.theme.clone(),
        image_protocol: cli.image_protocol.clone(),
        kitty_store: None,
    };

    let pager_forced = cli.pager || invoked_as_mdless;

    match &cli.command {
        Some(Commands::Completions { shell }) => {
            completions::print_completions(shell);
            Ok(())
        }
        Some(Commands::Render { files }) => {
            let files = files.clone();
            run(files, config, pager_forced)
        }
        None => {
            let files = cli.files.clone();
            run(files, config, pager_forced)
        }
    }
}

/// Pick the render width: an explicit `--width` wins, then a usable `$COLUMNS`,
/// then the detected terminal width.
///
/// `$COLUMNS` is often set by shells and scripts without being validated, so an
/// empty, non-numeric or zero value is ignored instead of aborting the program.
fn resolve_width(flag: Option<u16>, columns_env: Option<&str>, detect: impl FnOnce() -> u16) -> u16 {
    flag.or_else(|| {
        columns_env
            .and_then(|v| v.trim().parse::<u16>().ok())
            .filter(|&w| w > 0)
    })
    .unwrap_or_else(detect)
}

fn run(files: Vec<PathBuf>, config: render::Config, use_pager: bool) -> Result<()> {
    let sources = input::collect(files)?;

    let protocol = terminal::detect_image_protocol(config.image_protocol.as_deref());

    // Use the Kitty pager when pager mode is requested, images are enabled,
    // and the terminal speaks the Kitty graphics protocol (Kitty or Ghostty).
    // This path preserves images across pagination by capturing them in a store
    // and re-placing them with cheap APC sequences on each frame.
    let use_kitty_pager = use_pager
        && config.images
        && matches!(protocol, terminal::ImageProtocol::Kitty);

    if use_kitty_pager {
        let store = Rc::new(RefCell::new(render::KittyImageStore::new()));
        let render_config = render::Config {
            kitty_store: Some(Rc::clone(&store)),
            ..config
        };
        let rendered = render::render_all(&sources, &render_config)?;
        let doc = render::build_kitty_document(&rendered, &store.borrow());
        let opts = kitty_pager::PagerOptions {
            term_width: terminal::width(),
            term_height: terminal::height(),
            cell_pixel_width: terminal::cell_pixel_width(),
            cell_pixel_height: terminal::cell_pixel_height(),
        };
        return kitty_pager::page(doc, opts);
    }

    // The minus pager only handles CSI sequences; Kitty APC and iTerm2 OSC
    // sequences would be stripped or printed as garbage. Disable images and
    // mermaid diagrams when using the pager so alt text / code blocks are
    // shown instead.
    let graphics_protocol_active = config.images && matches!(
        protocol,
        terminal::ImageProtocol::Kitty | terminal::ImageProtocol::ITerm2
    );

    let render_config = if use_pager && graphics_protocol_active {
        render::Config {
            images: false,
            mermaid: false,
            ..config
        }
    } else {
        config
    };

    let rendered = render::render_all(&sources, &render_config)?;

    if use_pager {
        pager::show(rendered)
    } else {
        print!("{}", rendered);
        Ok(())
    }
}

mod completions {
    pub fn print_completions(shell: &str) {
        // TODO: implement via clap_complete
        eprintln!("Completions for {shell} not yet implemented");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_flag_takes_precedence_over_columns() {
        assert_eq!(resolve_width(Some(50), Some("120"), || 80), 50);
    }

    #[test]
    fn valid_columns_is_used_when_no_flag() {
        assert_eq!(resolve_width(None, Some("120"), || 80), 120);
        assert_eq!(resolve_width(None, Some(" 100 "), || 80), 100);
    }

    #[test]
    fn invalid_columns_falls_back_to_detected_width() {
        for bad in ["", "abc", "0", "-5", "99999999"] {
            assert_eq!(resolve_width(None, Some(bad), || 77), 77, "COLUMNS={bad:?}");
        }
    }

    #[test]
    fn missing_columns_uses_detected_width() {
        assert_eq!(resolve_width(None, None, || 77), 77);
    }

    #[test]
    fn width_flag_rejects_zero_and_garbage() {
        assert!(Cli::try_parse_from(["mdcat", "--width", "0"]).is_err());
        assert!(Cli::try_parse_from(["mdcat", "--width", "abc"]).is_err());
        let cli = Cli::try_parse_from(["mdcat", "--width", "40"]).unwrap();
        assert_eq!(cli.width, Some(40));
    }
}
