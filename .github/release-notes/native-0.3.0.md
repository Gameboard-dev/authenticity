The native programs of Authenticity 0.3.0, for Windows (x86-64, a processor from about 2015 on: AVX2).

| File | What it is |
|---|---|
| `authenticity-helper.exe` | Starts and stops SlopTotal for the AI-writing check. Put it in `<vault>/.obsidian/plugins/authenticity/` |
| `authenticity-mcp.exe` | The MCP server for research assistants (optional) |
| `check-overlap.exe` | The close-copying check of a draft from the command line (optional) |
| `*.sha256` | Each program's SHA-256 checksum |
| `authenticity-native-LICENSE.txt`, `authenticity-native-THIRD-PARTY-LICENSES.txt` | Authenticity's licence (MIT) and the licences of the crates the programs are built from |

Built by GitHub Actions from this tag and attested: `gh attestation verify authenticity-helper.exe --repo Gameboard-dev/authenticity`. Installation: [docs/installation.md](https://github.com/Gameboard-dev/authenticity/blob/main/docs/installation.md); the assistant tools: [docs/assistant-tools.md](https://github.com/Gameboard-dev/authenticity/blob/main/docs/assistant-tools.md).
