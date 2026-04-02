[![version](https://img.shields.io/badge/version-0.1.3-blue)](https://github.com/pavel-voronin/fz1)

# fz1

`fz1` is a terminal catalog and picker for CLI tools.

It loads a filesystem catalog, lets you browse it as a tree or fuzzy-search it like `fzf`, shows descriptions in a second pane, and prints the selected value to `stdout` instead of executing it.

## Demo

![fz1 demo](./demo.gif)

## Install

From crates.io:

```bash
cargo install fz1
```

From GitHub Releases:

1. Download the archive for your platform from the [latest release](https://github.com/pavel-voronin/fz1/releases/latest)
2. Extract the `fz1` binary
3. Put it somewhere on your `PATH`

From source:

```bash
git clone https://github.com/pavel-voronin/fz1
cd fz1
cargo install --path .
```

## Shell Integration

`fz1` renders the TUI on the terminal and prints only the selected value to `stdout`, so shell wrappers can insert it into the current prompt.

Source one of the included scripts:

```bash
source /path/to/fz1/shell/fz1.zsh
source /path/to/fz1/shell/fz1.bash
source /path/to/fz1/shell/fz1.fish
```

Default key binding in each script: `Ctrl+X g`.

## Usage

```text
Terminal catalog and picker for CLI tools

Usage: fz1 [OPTIONS]

Options:
      --catalog-dir <CATALOG_DIR>  Path to the catalog directory
      --print-catalog-dir          Print the resolved catalog directory and exit
      --no-enrich                  Disable async description enrichment
  -h, --help                       Print help
  -V, --version                    Print version

Default Catalog:
  $XDG_DATA_HOME/fz1/catalog or ~/.local/share/fz1/catalog
```

## Catalog Format

The catalog is a directory tree: directories are categories, files are entries. See [demo-catalog/](./demo-catalog)

- `= display name`: optional display-name override
- Any other non-`> cmd` lines are description
- `@ value`: selectable return template
- `@(label) value`: labeled return template
- `> command`: async enrichment command

Minimal example:

```text
catalog/
  git
  network/
    curl
```

`curl` file:

```text
= CURL
Tags: network, api
curl, you know

> curl --help

@ curl
@(Headers only) curl -I https://example.com
```

## License

MIT. See [LICENSE](LICENSE).
