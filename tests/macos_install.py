#!/usr/bin/env python3
"""macOS installer filesystem QA; Linux uses a fixture uname, not a native build.
Run: python3 tests/macos_install.py [path/to/native/macos/hfx]
All installation paths and HOME stay in a temporary workspace directory.
"""
import hashlib
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
NATIVE = Path(sys.argv.pop(1)).resolve() if len(sys.argv) > 1 else None


class InstallTests(unittest.TestCase):
    def setUp(self):
        (ROOT / "target").mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="mac-install-qa-", dir=ROOT / "target")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.home = self.root / "home space 日本語 $"
        self.home.mkdir()
        self.tools = self.root / "tools"
        self.tools.mkdir()
        if sys.platform != "darwin":
            self.executable(self.tools / "uname", '#!/bin/sh\nif [ "$1" = -s ]; then echo Darwin; else exec /usr/bin/uname "$@"; fi\n')
        self.env = dict(os.environ, HOME=str(self.home), PATH=f"{self.tools}:{os.environ['PATH']}")
        self.env.pop("CARGO_BUILD_TARGET", None)
        self.binary = self.root / "binary space $"
        self.executable(self.binary, '#!/bin/sh\npwd\nprintf "%s\\n" "$@"\n')
        self.app = self.home / "Applications/hfx.app"
        self.cli = self.home / ".local/bin/hfx"
        self.desktop = self.home / "Desktop/hfx.app"

    @staticmethod
    def executable(path, text):
        path.write_text(text)
        path.chmod(0o755)

    def run_script(self, name, *args, success=True):
        result = subprocess.run([str(ROOT / "scripts" / name), *map(str, args)], cwd=self.root,
                                env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def install(self, *args, success=True):
        return self.run_script("install-macos.sh", "--binary", self.binary, *args, success=success)

    def test_bundle_links_update_and_data_preserving_uninstall(self):
        self.install("--desktop-shortcut")
        contents = self.app / "Contents"
        plist = plistlib.loads((contents / "Info.plist").read_bytes())
        self.assertEqual(plist["CFBundleIdentifier"], "dev.horrified.hfx")
        self.assertEqual(plist["CFBundleExecutable"], "hfx")
        self.assertEqual(plist["CFBundleIconFile"], "hfx.icns")
        self.assertEqual((contents / "Resources/hfx.icns").read_bytes(), (ROOT / "assets/hfx.icns").read_bytes())
        self.assertEqual(self.cli.readlink(), contents / "MacOS/hfx-bin")
        self.assertEqual(self.desktop.readlink(), self.app)
        arguments = ["argument with spaces", 'quote " and $dollar']
        launch = subprocess.run([str(contents / "MacOS/hfx"), *arguments], env=self.env, cwd=self.root,
                                check=True, capture_output=True, text=True)
        workspace = self.home / "Library/Application Support/hfx/workspace"
        self.assertEqual(launch.stdout.splitlines(), [str(workspace), *arguments])
        terminal = subprocess.run([str(self.cli), *arguments], env=self.env, cwd=self.root,
                                  check=True, capture_output=True, text=True)
        self.assertEqual(terminal.stdout.splitlines(), [str(self.root), *arguments])
        data = workspace.parent
        (data / "chats.json").write_text("preserve chats")
        (data / "codex-auth.json").write_text("fixture credentials")
        (workspace / "user-file").write_text("preserve workspace")
        self.executable(self.binary, '#!/bin/sh\necho updated\n')
        self.install()  # The earlier Desktop shortcut survives omission of the flag.
        self.assertTrue(self.desktop.is_symlink())
        self.assertEqual((contents / "MacOS/hfx-bin").read_bytes(), self.binary.read_bytes())
        self.assertFalse(list(self.app.parent.glob(".hfx-install-*")))
        self.run_script("uninstall-macos.sh")
        self.assertFalse(self.app.exists())
        self.assertFalse(self.cli.is_symlink())
        self.assertFalse(self.desktop.is_symlink())
        self.assertEqual((data / "chats.json").read_text(), "preserve chats")
        self.assertEqual((data / "codex-auth.json").read_text(), "fixture credentials")
        self.assertEqual((workspace / "user-file").read_text(), "preserve workspace")

    def test_custom_paths_no_build_and_cargo_target_directory(self):
        apps = self.home / "custom apps"
        bins = self.home / "bin [custom]"
        target = self.root / "cargo target"
        (target / "release").mkdir(parents=True)
        self.executable(target / "release/hfx", self.binary.read_text())
        self.env["CARGO_TARGET_DIR"] = str(target)
        self.run_script("install-macos.sh", "--no-build", "--app-dir", apps, "--bin-dir", bins)
        self.assertTrue((apps / "hfx.app/Contents/MacOS/hfx-bin").is_file())
        self.assertTrue((bins / "hfx").is_symlink())
        self.run_script("uninstall-macos.sh", "--app-dir", apps, "--bin-dir", bins)

    def test_foreign_files_and_symlinks_are_not_overwritten(self):
        self.app.mkdir(parents=True)
        foreign = self.app / "user-file"
        foreign.write_text("not hfx")
        self.install(success=False)
        self.run_script("uninstall-macos.sh", success=False)
        self.assertEqual(foreign.read_text(), "not hfx")
        foreign.unlink()
        self.app.rmdir()
        self.cli.parent.mkdir(parents=True)
        self.cli.write_text("foreign cli")
        self.install(success=False)
        self.assertEqual(self.cli.read_text(), "foreign cli")
        self.cli.unlink()
        outside = self.root / "other.app"
        outside.mkdir()
        self.app.symlink_to(outside, target_is_directory=True)
        self.install(success=False)
        self.run_script("uninstall-macos.sh", success=False)
        self.assertEqual(self.app.readlink(), outside)
        self.app.unlink()
        self.desktop.parent.mkdir()
        self.desktop.write_text("foreign desktop")
        self.install("--desktop-shortcut", success=False)
        self.assertEqual(self.desktop.read_text(), "foreign desktop")

    def test_uninstall_preserves_an_unrelated_replacement_link(self):
        self.install("--desktop-shortcut")
        self.cli.unlink()
        self.cli.write_text("user replacement")
        self.run_script("uninstall-macos.sh")
        self.assertEqual(self.cli.read_text(), "user replacement")

    def test_failed_publication_rolls_back_app_and_new_links(self):
        self.executable(self.tools / "ln", '#!/bin/sh\nfor last in "$@"; do :; done\nif [ "$last" = "$HOME/Desktop/hfx.app" ]; then exit 79; fi\nexec /bin/ln "$@"\n')
        self.install("--desktop-shortcut", success=False)
        self.assertFalse(self.app.exists())
        self.assertFalse(self.cli.is_symlink())
        self.assertFalse(list(self.app.parent.glob(".hfx-install-*")))
        self.install()
        before = (self.app / "Contents/MacOS/hfx-bin").read_bytes()
        self.executable(self.binary, '#!/bin/sh\necho replacement\n')
        self.install("--desktop-shortcut", success=False)
        self.assertEqual((self.app / "Contents/MacOS/hfx-bin").read_bytes(), before)
        self.assertTrue(self.cli.is_symlink())
        self.assertFalse(self.desktop.is_symlink())

    def test_invalid_options_missing_binary_and_platform_guard(self):
        self.run_script("install-macos.sh", "--binary", success=False)
        self.run_script("install-macos.sh", "--binary", self.root / "missing", success=False)
        self.install("--app-dir", "relative", success=False)
        self.install("--bin-dir", "/", success=False)
        self.install("--app-dir", "////", success=False)
        self.install("--app-dir", str(self.home / "../apps"), success=False)
        self.install("--unknown", success=False)
        if sys.platform != "darwin":
            self.executable(self.tools / "uname", '#!/bin/sh\necho Linux\n')
            self.install(success=False)
            self.run_script("uninstall-macos.sh", success=False)

    @unittest.skipUnless(NATIVE is not None and sys.platform == "darwin", "requires a native macOS binary")
    def test_native_binary_and_plist(self):
        self.run_script("install-macos.sh", "--binary", NATIVE)
        installed = self.app / "Contents/MacOS/hfx-bin"
        self.assertEqual(hashlib.sha256(installed.read_bytes()).digest(), hashlib.sha256(NATIVE.read_bytes()).digest())
        subprocess.run(["plutil", "-lint", str(self.app / "Contents/Info.plist")], check=True)
        subprocess.run(["sips", "-g", "pixelWidth", str(self.app / "Contents/Resources/hfx.icns")], check=True)
        for executable in [self.cli, self.app / "Contents/MacOS/hfx"]:
            output = subprocess.check_output([str(executable), "--version"], env=self.env, text=True)
            self.assertTrue(output.startswith("hfx "))
        self.run_script("uninstall-macos.sh")


if __name__ == "__main__":
    unittest.main(verbosity=2)
