#!/usr/bin/env python3
"""Headless installer QA. Run: python3 tests/linux_install.py target/release/hfx"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/release/hfx").resolve()
if not BINARY.is_file():
    raise SystemExit("Build hfx first, or pass the executable path.")

with tempfile.TemporaryDirectory(prefix="install-qa-", dir=ROOT / "target") as temporary:
    root = Path(temporary)
    home, data, config, binary_dir = (root / name for name in ("home space", "data space", "config", 'bin %f " dollar$'))
    config.mkdir()
    home.mkdir()
    (config / "user-dirs.dirs").write_text('XDG_DESKTOP_DIR="$HOME/Bureau personnel"\n')
    env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=str(data), XDG_CONFIG_HOME=str(config), XDG_BIN_HOME=str(binary_dir), HFX_NO_DESKTOP_INTEGRATION="1")
    command = [str(ROOT / "scripts/install-linux.sh"), "--binary", str(BINARY), "--desktop-shortcut"]
    subprocess.run(command, env=env, check=True)
    entry = data / "applications/hfx.desktop"
    shortcut = home / "Bureau personnel/hfx.desktop"
    if shutil.which("desktop-file-validate"):
        subprocess.run(["desktop-file-validate", str(entry), str(shortcut)], check=True)
    assert shortcut.stat().st_mode & 0o777 == 0o755
    assert (binary_dir / "hfx").is_file()
    assert "%%f" in entry.read_text()
    (data / "hfx/chats.json").write_text("preserve chats")
    (data / "hfx/codex-auth.json").write_text("fake credential fixture")
    # A repeat install is safe; omitting the shortcut flag updates an existing one.
    subprocess.run(command, env=env, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(command[:-1], env=env, check=True, stdout=subprocess.DEVNULL)
    subprocess.run([str(ROOT / "scripts/uninstall-linux.sh")], env=env, check=True)
    assert not entry.exists() and not shortcut.exists() and not (binary_dir / "hfx").exists()
    assert (data / "hfx/chats.json").read_text() == "preserve chats"
    assert (data / "hfx/codex-auth.json").read_text() == "fake credential fixture"
    (binary_dir / "hfx").write_text("foreign executable")
    result = subprocess.run(command, env=env, capture_output=True, text=True)
    assert result.returncode != 0 and (binary_dir / "hfx").read_text() == "foreign executable"
print("Linux installer QA passed (temporary home only).")
