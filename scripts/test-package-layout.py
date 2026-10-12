"""Verify the shipped module archive layout without compiling native code."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent

class PackageTests(unittest.TestCase):
    def test_verifier_can_run_as_the_workspace_owner_with_a_prebuilt_runner(self):
        """Run module admission as the UID that owns a bind-mounted checkout."""
        target = ROOT / "target"; target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as directory:
            root = Path(directory)
            archive = root / "module.tar.gz"
            with tarfile.open(archive, "w:gz") as package:
                import io
                for name, data in [
                    ("libtinymcp_module.so", b"fixture"),
                    ("modules.toml", b'"libtinymcp_module.so" = "fixture-hash"\n'),
                ]:
                    info = tarfile.TarInfo(f"./{name}")
                    info.size = len(data)
                    package.addfile(info, io.BytesIO(data))

            observed = root / "observed"
            setpriv_args = root / "setpriv-args"
            bin_dir = root / "bin"
            bin_dir.mkdir()
            setpriv = bin_dir / "setpriv"
            setpriv.write_text(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$SET_PRIV_ARGS\"\n"
                "while [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"\n"
            )
            setpriv.chmod(0o755)
            uname = bin_dir / "uname"
            uname.write_text("#!/bin/sh\necho Darwin\n")
            uname.chmod(0o755)
            runner = root / "verify-module-fixture"
            runner.write_text(
                "#!/bin/sh\nprintf '%s\\n' \"$(id -u)\" > \"$OBSERVED_UID\"\n"
                "test -f \"$1\"\n"
            )
            runner.chmod(0o755)
            uid = os.getuid() + 1
            gid = os.getgid() + 1
            env = dict(
                os.environ,
                OBSERVED_UID=str(observed),
                SET_PRIV_ARGS=str(setpriv_args),
                PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
            )
            verifier = subprocess.run(
                [str(ROOT / "scripts/run-as-host-user.sh"), str(uid), str(gid),
                 str(ROOT / "scripts/verify-module.sh"),
                 "--archive", str(archive), "--verifier-bin", str(runner)],
                cwd=ROOT, env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(verifier.returncode, 0, verifier.stderr)
            self.assertEqual(observed.read_text().strip(), str(os.getuid()))
            self.assertEqual(
                setpriv_args.read_text().splitlines()[:4],
                [f"--reuid={uid}", f"--regid={gid}", "--clear-groups", "--"],
            )

    def test_archive_accepts_dotted_release_host_key_and_attests_library(self):
        """Package a dotted native host key and verify bytes plus attestation."""
        target = ROOT / "target"; target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as directory:
            root = Path(directory); (root/"scripts").mkdir(); (root/"bin").mkdir()
            shutil.copy2(ROOT/"scripts/build-module.sh", root/"scripts/build-module.sh")
            for file in ["LICENSE","README.md","MODULE.md"]: (root/file).write_text("fixture")
            cargo = root/"bin/cargo"
            cargo.write_text("#!/usr/bin/env python3\nimport json,sys\nfrom pathlib import Path\n"
                "root=Path.cwd()\n"
                "if 'metadata' in sys.argv: print(json.dumps({'target_directory':str(root/'target')}))\n"
                "else:\n"
                " target=sys.argv[sys.argv.index('--target')+1]\n"
                " output=root/'target'/target/'release'; output.mkdir(parents=True,exist_ok=True)\n"
                " (output/'libtinymcp.so').write_bytes(b'compiled-fixture')\n")
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=str(root/"bin")+os.pathsep+os.environ["PATH"])
            env.pop("CARGO_TARGET_DIR", None)
            alias = root/"checkout-link"; alias.symlink_to(root, target_is_directory=True)
            result = subprocess.run(["bash",str(alias/"scripts/build-module.sh"),
                "--crate","tinymcp","--version","1.2.3-beta.1",
                "--target","x86_64-unknown-linux-gnu","--host","ubuntu-24.04-x86_64"],
                cwd=root,env=env,text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            archive=root/"release-assets/tinymcp-1.2.3-beta.1-ubuntu-24.04-x86_64.tar.gz"
            with tarfile.open(archive) as package:
                data=package.extractfile("./libtinymcp_module.so").read()
                manifest=tomllib.loads(package.extractfile("./modules.toml").read().decode())
                self.assertEqual(data,b"compiled-fixture")
                self.assertEqual(manifest,{"libtinymcp_module.so":hashlib.sha256(data).hexdigest()})
                for name in ["./LICENSE","./README.md","./MODULE.md"]:
                    self.assertIsNotNone(package.extractfile(name))

    def test_arch_package_install_disables_only_landlock_filesystem_sandbox(self):
        """The nested distro container cannot apply pacman's Landlock ruleset."""
        (ROOT / "target").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            root = Path(directory)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            bash = shutil.which("bash")
            self.assertIsNotNone(bash)
            (bin_dir / "bash").symlink_to(bash)
            capture = root / "pacman-args"
            pacman = bin_dir / "pacman"
            pacman.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$PACMAN_ARGS\"\n")
            pacman.chmod(0o755)
            env = {"PATH": str(bin_dir), "PACMAN_ARGS": str(capture)}
            result = subprocess.run(
                [str(ROOT / "scripts/install-distro-packages.sh")],
                cwd=ROOT,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                capture.read_text().splitlines(),
                ["-Syu", "--disable-sandbox-filesystem", "--noconfirm", "base-devel", "cmake",
                 "curl", "git", "gzip", "jq", "openssl", "perl", "util-linux"],
            )

if __name__ == "__main__": unittest.main()
