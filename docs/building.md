# Building from source

[← README](../README.md)

Authenticity is written in Rust. The plugin is compiled to WebAssembly and inlined in `main.js`; the helper, the MCP
server and check-overlap are native programs.

## The workspace

This repository is one plugin of a Rust workspace: clone
[obsidian-cargo-lib](https://github.com/Gameboard-dev/obsidian-cargo-lib) (the shared crates and the build tool, pack),
then clone this repository into it as `authenticity`, and build from the workspace folder.

```bash
git clone https://github.com/Gameboard-dev/obsidian-cargo-lib
git clone https://github.com/Gameboard-dev/authenticity obsidian-cargo-lib/authenticity
```

## The plugin

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129     # must match the wasm-bindgen crate (core/Cargo.toml)
cd obsidian-cargo-lib
cargo run -p pack --release -- authenticity --release <dir>   # the release's main.js (module inlined), manifest.json, styles.css
cargo test --release -p authenticity-core -p authenticity-similar
```

Copy the three files into `<vault>/.obsidian/plugins/authenticity/`, then turn the plugin off and on in Obsidian.

## The native programs

The native programs are a workspace of their own (`native/`), since the plugin's workspace builds WebAssembly:

```bash
cd obsidian-cargo-lib/authenticity/native
cargo build --release      # target/release: authenticity-helper, authenticity-mcp, check-overlap
cargo test --release
```

Some of the tests start a local server on `127.0.0.1`; a sandbox that forbids that makes them fail.

## Releases

A plugin release is built by GitHub Actions from the tagged commit (`.github/workflows/release.yml`): the workspace at
a pinned commit, the tests, `pack --release` with the build computer's paths removed from the module, a check that no
such path is left, build-provenance attestations, and a release with exactly `main.js`, `manifest.json` and
`styles.css`. The native programs are released apart, as `native-x.y.z`, built for Windows (x86-64) by
`.github/workflows/native.yml`, with their `.sha256` files, attestations, `LICENSE` and third-party licences. Check a
downloaded file with:

```bash
gh attestation verify main.js --repo Gameboard-dev/authenticity
```

## Code layout

| Folder | What it holds |
|---|---|
| `core/` | The plugin (WebAssembly): the settings, the panes, the checks, the AI results, the hints, the characters, the GPTZero client, the log pane |
| `similar/` | The close-copying matcher: the library index, the copied-wording search and the content-word paraphrase search |
| `native/` | The helper, the MCP server and check-overlap |
| `licences/` | The third-party licence files and the script that writes them ([Licences](licences.md)) |

`cargo doc -p authenticity-core --no-deps --open` builds the code's documentation.
