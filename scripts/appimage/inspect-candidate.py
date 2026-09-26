"""Inspect the actual generated AppImage; never publish or modify it."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
TOOLCHAIN = json.loads(Path(__file__).with_name("toolchain.json").read_text())
TAURI_REVISION = TOOLCHAIN["tauriRevision"]


def run(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inspect(app_dir, arch):
    libraries = sorted(str(p.relative_to(app_dir)) for p in app_dir.rglob("*.so*"))
    # linuxdeploy's upstream exclusion targets the client ABI used by host Mesa.
    # Other Wayland libraries remain in the upstream bundle; inventory them for
    # desktop testing rather than treating their presence alone as a proven bug.
    wayland_libraries = [p for p in libraries if re.match(r"libwayland-.*\.so", Path(p).name)]
    conflicts = [p for p in wayland_libraries if re.match(r"libwayland-client\.so", Path(p).name)]
    if conflicts:
        raise RuntimeError(f"AppImage still shadows the host Wayland client: {conflicts}")
    hook = app_dir / "apprun-hooks/linuxdeploy-plugin-gtk.sh"
    hook_text = hook.read_text()
    if not re.search(r"^export GIO_MODULE_DIR=", hook_text, re.M):
        raise RuntimeError("Missing bundled GIO module selection")
    if re.search(r"^\s*export GDK_BACKEND=x11", hook_text, re.M):
        raise RuntimeError("GTK hook still forces X11")
    gio_modules = list((app_dir / "usr/lib/gio/modules").glob("*.so"))
    if not gio_modules or not (app_dir / "usr/lib/gio/modules/giomodule.cache").is_file():
        raise RuntimeError("Bundled GIO modules or their cache are missing")

    binaries = [app_dir / "usr/bin/llama-server-manager"]
    for name in ("WebKitWebProcess", "WebKitNetworkProcess"):
        matches = sorted({p.resolve() for p in app_dir.rglob(name) if p.is_file()})
        if len(matches) != 1:
            raise RuntimeError(f"Expected one {name}, found {matches}")
        binaries.extend(matches)
    dependencies = {}
    for binary in binaries + gio_modules:
        with binary.open("rb") as stream:
            header = stream.read(20)
        if header[:4] != b"\x7fELF" or struct.unpack("<H", header[18:20])[0] != {"x86_64": 62, "aarch64": 183}[arch]:
            raise RuntimeError(f"Wrong ELF architecture: {binary}")
        environment = dict(os.environ, LD_LIBRARY_PATH=str(app_dir / "usr/lib"))
        output = run("ldd", str(binary), env=environment)
        if "not found" in output:
            raise RuntimeError(f"Unresolved dependencies in {binary}:\n{output}")
        dependencies[str(binary.relative_to(app_dir))] = output.replace(str(app_dir), "$APPDIR")
    return {
        "bundled_libraries": libraries,
        "remaining_wayland_libraries_for_runtime_review": wayland_libraries,
        "dependencies": dependencies,
        "gtk_hook": hook_text,
    }


def main():
    os.chdir(ROOT)
    arch = os.environ["CANDIDATE_ARCH"]
    candidates = list((ROOT / "src-tauri/target/release/bundle/appimage").glob("*.AppImage"))
    if len(candidates) != 1:
        raise RuntimeError(f"Expected one final AppImage, found {candidates}")
    candidate = candidates[0]
    with candidate.open("rb") as stream:
        header = stream.read(12)
    if header[:4] != b"\x7fELF" or header[8:11] != b"AI\x02":
        raise RuntimeError("Not a type-2 AppImage")
    output_dir = ROOT / ".temp/appimage-candidate"
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="lsm-appimage-inspect-") as temporary:
        with (output_dir / "extraction.log").open("w") as log:
            subprocess.run([str(candidate), "--appimage-extract"], cwd=temporary, stdout=log, stderr=subprocess.STDOUT, check=True)
        audit = inspect(Path(temporary) / "squashfs-root", arch)
    upstream = ROOT / ".temp/appimage-tauri"
    if run("git", "-C", str(upstream), "rev-parse", "HEAD") != TAURI_REVISION:
        raise RuntimeError("Unexpected Tauri bundler source revision")
    source_commit = run("git", "rev-parse", "HEAD")
    version = json.loads((ROOT / "package.json").read_text())["version"]
    filename = f"LlamaServerManager_{version}_appimage-candidate_{source_commit[:12]}_{arch}.AppImage"
    shutil.copy2(candidate, output_dir / filename)
    audit.update({
        "application_commit": source_commit,
        "application_version": version,
        "architecture": arch,
        "tauri_bundler_commit": TAURI_REVISION,
        "rustc": run("rustc", "--version"),
        "build_os": Path("/etc/os-release").read_text(),
        "runtime_graphics_validation": "PENDING: user validation on Ubuntu 24.04/26.04",
        "updater_artifacts": False,
        "reviewed_toolchain": TOOLCHAIN,
        "tool_sha256": {p.name: sha256(p) for p in sorted((Path.home() / ".cache/tauri").glob("*")) if p.is_file()},
    })
    (output_dir / "build-info.json").write_text(json.dumps(audit, indent=2) + "\n")
    shutil.copy2(ROOT / "docs/APPIMAGE_CANDIDATE.md", output_dir / "README.md")
    shutil.copy2(ROOT / "scripts/appimage/run-with-diagnostics.sh", output_dir)
    (output_dir / "SHA256SUMS").write_text("".join(f"{sha256(p)}  {p.name}\n" for p in sorted(output_dir.iterdir()) if p.is_file() and p.name != "SHA256SUMS"))
    print(f"Inspected {filename}; graphical validation remains pending.")


if __name__ == "__main__":
    main()
