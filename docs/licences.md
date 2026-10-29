# Licences

[← README](../README.md)

## Authenticity

The plugin and its native programs are released under the MIT licence ([LICENSE](../LICENSE)).

## Third-party code

The third-party licences of the Rust crates the plugin and the native programs are built from are in `licences/`:
[`plugin.txt`](../licences/plugin.txt) and [`native.txt`](../licences/native.txt). Each native release carries
`native.txt` as `authenticity-native-THIRD-PARTY-LICENSES.txt`, with Authenticity's `LICENSE`. After a dependency
changes, regenerate them with `python licences/generate.py` (it needs Python 3.8+ and cargo-about).

## SlopTotal and its models

SlopTotal is not distributed with Authenticity; you install it yourself ([Installation](installation.md#3-sloptotal)).
Its code is under the MIT licence. The models it downloads keep their own licences: most are MIT or Apache-2.0. Three
are different:

| Model | Licence |
|---|---|
| SuperAnnotate's detector (`SuperAnnotate/ai-detector-low-fpr`) | Its own SAIPL v1.0 licence, copyleft with an attribution requirement |
| The ChatGPT detector (`Hello-SimpleAI/chatgpt-detector-roberta`) | No licence stated |
| ReMoDetect (`hyunseoki/ReMoDetect-deberta`) | No licence stated |

You download them for your own use.

## GPTZero

GPTZero is a commercial online service under its own [terms of use](https://gptzero.me/terms-of-use.html). Authenticity
includes none of its code; it sends requests to GPTZero's public API only when you choose it.
