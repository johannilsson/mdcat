use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod input;
mod pager;
mod render;
mod terminal;

#[derive(Parser)]
#[command(
    name = "mdcat",
    about = "Cat for markdown - render formatted markdown in the terminal",
    version,
    after_help = "When invoked as 'mdless', pager mode is enabled automatically.\n\nExamples:\n  mdcat README.md\n  mdcat file1.md file2.md\n  cat README.md | mdcat\n  mdcat --pager README.md"
)]
struct Cli {
    /// Files to render (reads from stdin if omitted)
    files: Vec<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,

    /// Force pager mode (like less)
    #[arg(long, conflicts_with = "no_pager")]
    pager: bool,

    /// Disable pager mode
    #[arg(long)]
    no_pager: bool,

    /// Terminal width override (defaults to detected width or 80)
    #[arg(long, env = "COLUMNS")]
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
        width: cli.width.unwrap_or_else(terminal::width),
        images: !cli.no_images,
        mermaid: !cli.no_mermaid,
        mermaid_binary: cli.mermaid_binary.clone(),
        theme: cli.theme.clone(),
        image_protocol: cli.image_protocol.clone(),
    };

    let use_pager = if cli.no_pager {
        false
    } else if cli.pager || invoked_as_mdless {
        true
    } else {
        // Auto: paginate if stdout is a TTY
        terminal::is_tty()
    };

    match &cli.command {
        Some(Commands::Completions { shell }) => {
            completions::print_completions(shell);
            Ok(())
        }
        Some(Commands::Render { files }) => {
            let files = files.clone();
            run(files, config, use_pager)
        }
        None => {
            let files = cli.files.clone();
            run(files, config, use_pager)
        }
    }
}

fn run(files: Vec<PathBuf>, config: render::Config, use_pager: bool) -> Result<()> {
    let sources = input::collect(files)?;
    let rendered = render::render_all(&sources, &config)?;

    // The minus pager only handles CSI sequences; it strips Kitty APC and iTerm2
    // OSC sequences, printing the raw base64 payload as text. Bypass the pager
    // whenever a graphics protocol is in use.
    let graphics_protocol = config.images && matches!(
        terminal::detect_image_protocol(config.image_protocol.as_deref()),
        terminal::ImageProtocol::Kitty | terminal::ImageProtocol::ITerm2
    );

    let needs_pager = use_pager
        && !graphics_protocol
        && rendered.lines().count() > terminal::height() as usize;

    if needs_pager {
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
