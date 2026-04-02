mod app;
mod catalog;
mod demo;
mod editor;
mod enrichment;
mod search;
mod state;
mod tree;
mod ui;
mod xdg;

use clap::Parser;
use std::path::PathBuf;

#[cfg(not(target_os = "windows"))]
const DEFAULT_CATALOG_DIR_SECTION: &str =
    "Default Catalog:\n  $XDG_DATA_HOME/fz1/catalog or ~/.local/share/fz1/catalog";

#[cfg(target_os = "windows")]
const DEFAULT_CATALOG_DIR_SECTION: &str =
    "Default Catalog:\n  %APPDATA%\\fz1\\catalog or %USERPROFILE%\\AppData\\Roaming\\fz1\\catalog";

#[derive(Parser)]
#[command(
    name = "fz1",
    about = "Terminal catalog and picker for CLI tools",
    version,
    after_help = DEFAULT_CATALOG_DIR_SECTION
)]
struct Cli {
    #[arg(long, help = "Path to the catalog directory")]
    catalog_dir: Option<PathBuf>,
    /// Print the resolved catalog directory and exit
    #[arg(long)]
    print_catalog_dir: bool,
    /// Disable async description enrichment
    #[arg(long)]
    no_enrich: bool,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let catalog_root = resolve_catalog_dir(cli.catalog_dir);
    if cli.print_catalog_dir {
        println!("{}", catalog_root.display());
        return Ok(());
    }
    std::fs::create_dir_all(&catalog_root)?;
    let entries = catalog::load_catalog(&catalog_root)?;
    let enrichment = if cli.no_enrich {
        None
    } else {
        Some(enrichment::spawn_enrichment(&entries))
    };
    let pane_split_percent = state::load_pane_split_percent();
    let mut app = app::App::new(catalog_root, entries, enrichment, pane_split_percent);
    if let Some(filename) = ui::run(&mut app)? {
        println!("{}", filename);
    }
    Ok(())
}

fn default_catalog_dir() -> PathBuf {
    xdg::data_home().join("fz1").join("catalog")
}

fn resolve_catalog_dir(catalog_dir: Option<PathBuf>) -> PathBuf {
    catalog_dir.unwrap_or_else(default_catalog_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_catalog_dir_prefers_explicit_flag() {
        let explicit = PathBuf::from("/tmp/custom-catalog");
        assert_eq!(resolve_catalog_dir(Some(explicit.clone())), explicit,);
    }

    #[test]
    fn cli_accepts_print_catalog_dir_flag() {
        let cli = Cli::try_parse_from(["fz1", "--print-catalog-dir"]).unwrap();
        assert!(cli.print_catalog_dir);
    }
}
