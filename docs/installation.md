# Installation

[← README](../README.md)

Authenticity has three parts: the plugin, a small helper program that starts and stops SlopTotal, and SlopTotal
itself, the local AI detector. Only the AI-writing check needs the helper and SlopTotal; the plagiarism check needs a
library folder, and Reveal AI characters needs nothing.

| Check | What it needs | Set-up |
|---|---|---|
| AI writing | The helper and SlopTotal | [The helper](#2-the-helper), [SlopTotal](#3-sloptotal): about 20 minutes and a 9 GB download, once |
| AI writing with GPTZero (optional) | A GPTZero API key | [GPTZero](#5-gptzero-optional) |
| Plagiarism and close paraphrase | A folder of your source notes | [The library folder](#4-the-library-folder) |
| Reveal AI characters | Nothing | None |

## 1. The plugin

Install **Authenticity** from Obsidian's community plugins, or copy `main.js`, `manifest.json` and `styles.css` from a
[release](https://github.com/Gameboard-dev/authenticity/releases) into `<vault>/.obsidian/plugins/authenticity/`, then
enable it under Settings → Community plugins.

## 2. The helper

Obsidian installs only the plugin's three files, so download the helper yourself from the
[native release](https://github.com/Gameboard-dev/authenticity/releases/tag/native-0.3.0):
`authenticity-helper.exe` (Windows, x86-64), with its `.sha256` file to check the download. Put it in the plugin's
folder (`<vault>/.obsidian/plugins/authenticity/`), or anywhere you like and set **Helper program** for this computer
([Settings](settings.md#writing-checks)). On other systems, build it from source ([Building](building.md)).

To check the download's provenance with the GitHub CLI:

```powershell
gh attestation verify authenticity-helper.exe --repo Gameboard-dev/authenticity
```

## 3. SlopTotal

[SlopTotal](https://github.com/pablocaeg/sloptotal) (MIT licence) is a local AI-text detector: 23 detectors (neural
classifiers and statistical tests) combined by a fitted model into one score per text, with a one-line reason from each
detector. Authenticity starts it in the background when you run a check, talks to it only on `127.0.0.1`, and stops it
after a few idle minutes. How it scores, and how far to trust it: [AI detection](ai-detection.md).

### What you need

- **Git** (`git --version` in PowerShell).
- **Python 3.10 or later** (`python --version`). If the install in step 3 says *No matching distribution found for
  torch*, your Python is newer than PyTorch's wheels: install Python 3.12 from <https://www.python.org/downloads/> and
  use its `python` in steps 2 and 3.
- **About 11 GB of disk** (9 GB of models, 2 GB of Python packages) and **about 9 GB of free memory** while a check
  runs.
- An internet connection **for the installation only**.

Install it **outside the vault and outside any synchronised folder**, so the 9 GB of models are not synced. The steps
use `%LOCALAPPDATA%\Programs\SlopTotal`.

### Step 1: download SlopTotal

In PowerShell:

```powershell
$dir = "$env:LOCALAPPDATA\Programs\SlopTotal"
git clone https://github.com/pablocaeg/sloptotal.git $dir
Set-Location $dir
```

### Step 2: make its own Python environment

```powershell
python -m venv venv
.\venv\Scripts\python.exe -m pip install --upgrade pip
```

The environment keeps SlopTotal's packages apart from anything else on the computer.

### Step 3: install its packages

```powershell
.\venv\Scripts\python.exe -m pip install -r requirements.txt
```

This installs the CPU build of PyTorch (about 2 GB), which every computer can run.

**Optional, for a large graphics card (an NVIDIA card with more than 9 GB):** SlopTotal's models take about 8.3 GB of
graphics memory, so on such a card the check is faster. Swap in PyTorch's CUDA build of the same version (SlopTotal's
README, "NVIDIA GPU"; for example `.\venv\Scripts\python.exe -m pip install --force-reinstall torch --index-url
https://download.pytorch.org/whl/cu121`). On a card of 8 GB or less, keep the CPU build: the models do not fit.

### Step 4: first start, which downloads the models (about 9 GB, once)

Still in the same PowerShell window:

```powershell
$env:SLOPTOTAL_DEVICE = "cpu"
$env:HF_HOME = "$dir\models"
$env:SLOPTOTAL_RETENTION_DAYS = "1"
.\venv\Scripts\python.exe -m uvicorn app.main:app --host 127.0.0.1 --port 8000
```

- `SLOPTOTAL_DEVICE=cpu`: the processor for this first start.
- `HF_HOME`: the models go into `SlopTotal\models`, where Authenticity points SlopTotal later.
- `SLOPTOTAL_RETENTION_DAYS=1`: SlopTotal keeps a copy of each text it checks in its own database (in
  `SlopTotal\data`); this keeps it for one day, the shortest it allows (0 would keep it for ever).
- `--host 127.0.0.1`: it answers only this computer. Never leave this out: SlopTotal's own scripts default to
  `0.0.0.0`, which answers the whole network.

The models download in the background from huggingface.co. Wait until SlopTotal's window says **"Model preloading
complete"**, then, in a **second** PowerShell window, check that every detector scores:

```powershell
Set-Location "$env:LOCALAPPDATA\Programs\SlopTotal"
.\venv\Scripts\python.exe scripts\smoke_test.py
```

SlopTotal's `/health` counts the detectors that have run so far, not those loaded, so do not wait for it to show all
of them. Then stop SlopTotal: press **Ctrl+C** in the first window.

### Step 5: tell Authenticity where it is

In **Settings → Authenticity → Writing checks**, set **SlopTotal folder** to the folder, as a full path or with
environment variables (`%LOCALAPPDATA%\Programs\SlopTotal` on Windows, `~/sloptotal` or `$HOME/sloptotal` elsewhere);
*Status* under it shows the folder they expand to. It is kept on this computer only, never in the vault. The device and
port settings are described in [Settings](settings.md#writing-checks).

From then on Authenticity starts SlopTotal itself when you check, as in step 4 but with downloads switched off
(`HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1`, `HF_HUB_DISABLE_TELEMETRY=1`). The first check of a session takes tens
of seconds while the models load.

## 4. The library folder

Set **Library folder** (Settings → Authenticity → Plagiarism) to the vault folder of your source notes, such as
`Sources/` or `Library/`; until it is set, the plagiarism check asks for it. The first check builds an index of the
library, kept on this computer outside the vault. What the check finds, and which notes are never treated as sources:
[Similarity](similarity.md).

## 5. GPTZero (optional)

[GPTZero](https://gptzero.me) is an online AI detector that Authenticity can use instead of SlopTotal for a check. It
is off until you link an API key.

1. Create an API key in your GPTZero account (see [GPTZero's developer page](https://gptzero.me/developers)). API
   access is a paid GPTZero plan with a monthly word allowance: every check you send to GPTZero counts the words of
   the text against it.
2. Settings → Authenticity → GPTZero → **GPTZero API key**: link it. The key is kept in Obsidian's keychain (Settings →
   Keychain), never in Authenticity's files.
3. **Check the key** asks GPTZero for your plan and the words left this cycle; it uses no words.

How the choice is asked, and what is sent: [AI detection](ai-detection.md#gptzero-optional) and
[Privacy and security](privacy-and-security.md#gptzero).

## 6. The optional embedding search

If the Quarry plugin is installed, Authenticity can optionally use its embedding model for the paraphrase check
(Settings → Plagiarism → *Embedding search*, off by default; *Check* tests it). Authenticity's own search needs no model
and works without it ([Similarity](similarity.md#the-optional-embedding-search)).

## Removing it

1. Disable and remove the plugin under Settings → Community plugins (this deletes its folder, the helper with it).
2. Delete Authenticity's local data folder: `%LOCALAPPDATA%\authenticity` on Windows (`~/.local/share/authenticity` and
   `~/.cache/authenticity` elsewhere), which holds the library index, the history and the logs.
3. Delete SlopTotal's folder if you no longer want it, and unlink the GPTZero key from Obsidian's keychain.
