#!/usr/bin/env python3
"""Create native release artifacts. Never installs, registers, or publishes them."""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import tarfile
from urllib.parse import urlparse
from xml.sax.saxutils import escape
import xml.etree.ElementTree as ET

HERE = Path(__file__).resolve().parent
DESKTOP = HERE.parent
ICON = DESKTOP / "crates/app/assets/icon.png"
LICENSE = DESKTOP.parent / "LICENSE"


def required(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise ValueError(f"Required environment variable: {name}")
    return value


def tool(name: str, override: str | None = None) -> str:
    candidate = os.environ.get(override, name) if override else name
    found = shutil.which(candidate)
    if not found:
        raise ValueError(f"Required tool: {name}" + (f" (or set {override})" if override else ""))
    return found


def run(*args: str | Path, env: dict | None = None) -> None:
    # Argument arrays also keep publisher names, paths and signing IDs out of a shell.
    subprocess.run([str(arg) for arg in args], check=True, env=env)


def version_parts(version: str) -> list[int]:
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:\.\d+)?", version):
        raise ValueError("--version must be three or four numeric components, for example 1.2.3")
    parts = [int(part) for part in version.split(".")]
    if any(part > 65535 for part in parts):
        raise ValueError("Version components must be at most 65535")
    return parts


def render(template: str, values: dict[str, str]) -> str:
    source = (HERE / template).read_text(encoding="utf-8")
    for name, value in values.items():
        source = source.replace(f"@{name}@", escape(value, {'"': "&quot;", "'": "&apos;"}))
    if re.search(r"@[A-Z_]+@", source):
        raise ValueError(f"Unresolved template variable in {template}")
    ET.fromstring(source)
    return source


def write_template(target: Path, template: str, values: dict[str, str]) -> None:
    target.write_text(render(template, values), encoding="utf-8")


def https_base(value: str) -> str:
    parsed = urlparse(value)
    if parsed.scheme != "https" or not parsed.netloc or parsed.query or parsed.fragment or parsed.username:
        raise ValueError("KOYORI_UPDATE_BASE_URL must be an HTTPS directory URL without credentials, query or fragment")
    return value.rstrip("/")


def binary_arch(binary: Path, system: str) -> str:
    with binary.open("rb") as source:
        header = source.read(64)
        if system == "win32" and header[:2] == b"MZ":
            source.seek(struct.unpack_from("<I", header, 60)[0])
            pe = source.read(6)
            if pe[:4] == b"PE\0\0":
                return {0x8664: "x64", 0xAA64: "arm64"}[struct.unpack_from("<H", pe, 4)[0]]
        elif system == "linux" and header[:4] == b"\x7fELF" and header[5] == 1:
            return {62: "x86_64", 183: "aarch64"}[struct.unpack_from("<H", header, 18)[0]]
        elif system == "darwin" and header[:4] == b"\xcf\xfa\xed\xfe":
            return {0x01000007: "x86_64", 0x0100000C: "arm64"}[struct.unpack_from("<I", header, 4)[0]]
    raise ValueError("Expected a native 64-bit executable for this OS (package macOS architectures separately)")


def windows(binary: Path, version: str, arch: str, out: Path, unsigned: bool) -> list[Path]:
    publisher = required("KOYORI_WINDOWS_PUBLISHER")
    publisher_name = required("KOYORI_WINDOWS_PUBLISHER_NAME")
    makeappx = tool("makeappx.exe", "KOYORI_MAKEAPPX")
    powershell = tool("powershell.exe")
    # Use the matching Visual C++ Redistributable directory, not development DLLs.
    runtime = Path(required("KOYORI_WINDOWS_RUNTIME_DIR")).resolve()
    if not (runtime / "vcruntime140.dll").is_file():
        raise ValueError("KOYORI_WINDOWS_RUNTIME_DIR must contain the matching redistributable vcruntime140.dll")
    if binary_arch(runtime / "vcruntime140.dll", "win32") != arch:
        raise ValueError("Visual C++ redistributable architecture does not match the executable")
    if not unsigned:
        signtool = tool("signtool.exe", "KOYORI_SIGNTOOL")
        thumbprint = required("KOYORI_WINDOWS_CERT_THUMBPRINT")
        timestamp = required("KOYORI_WINDOWS_TIMESTAMP_URL")
        if urlparse(timestamp).scheme not in ("https", "http"):
            raise ValueError("KOYORI_WINDOWS_TIMESTAMP_URL must be an RFC 3161 timestamp service URL")
        feed_base = https_base(required("KOYORI_UPDATE_BASE_URL"))

    package_version = ".".join(map(str, (version_parts(version) + [0])[:4]))
    stage = out / "_staging" / "package"
    stage.mkdir(parents=True)
    shutil.copy2(binary, stage / "koyori.exe")
    shutil.copy2(LICENSE, stage / "LICENSE")
    for dll in runtime.glob("*.dll"):
        shutil.copy2(dll, stage / dll.name)
    run(powershell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
        HERE / "windows/resize-icons.ps1", "-Source", ICON, "-Destination", stage / "Assets")
    values = {"PUBLISHER": publisher, "PUBLISHER_NAME": publisher_name,
              "VERSION": package_version, "ARCH": arch}
    write_template(stage / "AppxManifest.xml", "windows/AppxManifest.xml.in", values)
    msix = out / f"Koyori-{version}-{arch}{'-unsigned' if unsigned else ''}.msix"
    if not unsigned:
        run(signtool, "sign", "/sha1", thumbprint, "/fd", "SHA256", "/tr", timestamp,
            "/td", "SHA256", stage / "koyori.exe")
    run(makeappx, "pack", "/d", stage, "/p", msix)
    if unsigned:
        return [msix]
    run(signtool, "sign", "/sha1", thumbprint, "/fd", "SHA256", "/tr", timestamp, "/td", "SHA256", msix)
    run(signtool, "verify", "/pa", "/v", msix)
    # A stable feed filename lets existing installations discover subsequent versions.
    feed = out / f"Koyori-{arch}.appinstaller"
    values.update(FEED_URL=f"{feed_base}/{feed.name}", PACKAGE_URL=f"{feed_base}/{msix.name}")
    write_template(feed, "windows/Koyori.appinstaller.in", values)
    return [msix, feed]


def macos(binary: Path, version: str, arch: str, out: Path, unsigned: bool) -> list[Path]:
    for name in ("sips", "iconutil", "hdiutil", "otool"):
        tool(name)
    parts = version_parts(version)
    if len(parts) == 4 and parts[-1] != 0:
        raise ValueError("macOS releases use three version components (a fourth component may only be zero)")
    bundle_version = ".".join(map(str, parts[:3]))
    if not unsigned:
        for name in ("codesign", "xcrun", "ditto", "spctl"):
            tool(name)
        identity = required("KOYORI_MACOS_SIGN_IDENTITY")
        profile = required("KOYORI_MACOS_NOTARY_PROFILE")
    dependencies = subprocess.check_output(["otool", "-L", str(binary)], text=True)
    for line in dependencies.splitlines()[1:]:
        dependency = line.strip().split(" (", 1)[0]
        if not dependency.startswith(("/System/Library/", "/usr/lib/")):
            raise ValueError(f"Unbundled macOS dependency: {dependency}. Build without external dylibs before packaging.")

    stage = out / "_staging"
    content = stage / "dmg"
    app = content / "Koyori.app"
    executable_dir = app / "Contents/MacOS"
    resources = app / "Contents/Resources"
    executable_dir.mkdir(parents=True)
    resources.mkdir(parents=True)
    shutil.copy2(binary, executable_dir / "koyori")
    (executable_dir / "koyori").chmod(0o755)
    shutil.copy2(LICENSE, resources / "LICENSE")
    write_template(app / "Contents/Info.plist", "macos/Info.plist.in",
                   {"SHORT_VERSION": bundle_version, "VERSION": bundle_version})
    iconset = stage / "Koyori.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            filename = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
            run("sips", "-z", str(size * scale), str(size * scale), ICON, "--out", iconset / filename)
    run("iconutil", "-c", "icns", iconset, "-o", resources / "Koyori.icns")
    (content / "Applications").symlink_to("/Applications")
    if not unsigned:
        run("codesign", "--force", "--options", "runtime", "--timestamp", "--sign", identity, app)
        run("codesign", "--verify", "--strict", "--verbose=2", app)
        archive = stage / "Koyori-notarization.zip"
        run("ditto", "-c", "-k", "--keepParent", app, archive)
        run("xcrun", "notarytool", "submit", archive, "--keychain-profile", profile, "--wait")
        run("xcrun", "stapler", "staple", app)
        run("spctl", "--assess", "--type", "execute", "--verbose=2", app)
    dmg = out / f"Koyori-{version}-{arch}{'-unsigned' if unsigned else ''}.dmg"
    run("hdiutil", "create", "-volname", "Koyori", "-srcfolder", content, "-ov", "-format", "UDZO", dmg)
    if not unsigned:
        run("codesign", "--timestamp", "--sign", identity, dmg)
        run("xcrun", "notarytool", "submit", dmg, "--keychain-profile", profile, "--wait")
        run("xcrun", "stapler", "staple", dmg)
        run("xcrun", "stapler", "validate", dmg)
    return [dmg]


def linux(binary: Path, version: str, arch: str, out: Path, unsigned: bool) -> list[Path]:
    deploy = tool("linuxdeploy", "KOYORI_LINUXDEPLOY")
    appimagetool = tool("appimagetool", "KOYORI_APPIMAGETOOL")
    runtime = Path(required("KOYORI_APPIMAGE_RUNTIME")).resolve()
    if not runtime.is_file():
        raise ValueError("KOYORI_APPIMAGE_RUNTIME must point to a local AppImage runtime")
    if binary_arch(runtime, "linux") != arch:
        raise ValueError("AppImage runtime architecture does not match the executable")
    if not unsigned:
        gpg = tool("gpg")
        key = required("KOYORI_GPG_KEY")
        run(gpg, "--batch", "--list-secret-keys", key)
    appdir = out / "_staging/Koyori.AppDir"
    bindir = appdir / "usr/bin"
    bindir.mkdir(parents=True)
    shutil.copy2(binary, bindir / "koyori")
    (bindir / "koyori").chmod(0o755)
    shutil.copy2(ICON, appdir / "koyori.png")
    shutil.copy2(LICENSE, appdir / "LICENSE")
    env = dict(os.environ, ARCH=arch, VERSION=version, APPIMAGE_EXTRACT_AND_RUN="1")
    run(deploy, "--appdir", appdir, "--executable", bindir / "koyori",
        "--desktop-file", HERE / "linux/app.koyori.desktop.desktop",
        "--icon-file", appdir / "koyori.png", env=env)
    appimage = out / f"Koyori-{version}-{arch}{'-unsigned' if unsigned else ''}.AppImage"
    command = [appimagetool, "--runtime-file", str(runtime)]
    if not unsigned:
        command += ["--sign", "--sign-key", key]
    run(*command, appdir, appimage, env=env)
    archive = out / f"Koyori-{version}-{arch}{'-unsigned' if unsigned else ''}.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(appdir, arcname="Koyori")
    artifacts = [appimage, archive]
    if not unsigned:
        for artifact in list(artifacts):
            signature = Path(f"{artifact}.asc")
            run(gpg, "--batch", "--armor", "--detach-sign", "--local-user", key,
                "--output", signature, artifact)
            run(gpg, "--batch", "--verify", signature, artifact)
            artifacts.append(signature)
    return artifacts


def self_test() -> None:
    values = {"PUBLISHER": 'CN=Test & "quoted"', "PUBLISHER_NAME": "Template test",
              "VERSION": "1.2.3.0", "ARCH": "x64", "FEED_URL": "https://example.invalid/Koyori-x64.appinstaller",
              "PACKAGE_URL": "https://example.invalid/Koyori-1.2.3-x64.msix"}
    manifest = ET.fromstring(render("windows/AppxManifest.xml.in", values))
    identity = manifest.find("{*}Identity")
    feed = ET.fromstring(render("windows/Koyori.appinstaller.in", values)).find("{*}MainPackage")
    for field in ("Name", "Publisher", "Version", "ProcessorArchitecture"):
        assert identity.attrib[field] == feed.attrib[field], field
    assert identity.attrib["Publisher"] == values["PUBLISHER"]
    plist = plistlib.loads(render("macos/Info.plist.in", {"SHORT_VERSION": "1.2.3", "VERSION": "1.2.3"}).encode())
    assert plist["CFBundleIdentifier"] == identity.attrib["Name"]
    for invalid in ("../1", "1.2", "1.2.3-beta", "1.2.65536", "1.2.3.4.5"):
        try:
            version_parts(invalid)
        except ValueError:
            continue
        raise AssertionError(f"Accepted invalid release version: {invalid}")
    assert ICON.is_file() and LICENSE.is_file()
    print("Release templates: XML escaping, MSIX/feed identity, plist, version validation and source assets passed")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", help="Release version, e.g. 1.2.3")
    parser.add_argument("--binary", type=Path, help="Native release executable; defaults to target/release/koyori[.exe]")
    parser.add_argument("--out", type=Path, help="New output directory; existing directories are rejected")
    parser.add_argument("--unsigned", action="store_true", help="Local packaging check only; no signing, notarization or update feed")
    parser.add_argument("--self-test", action="store_true", help="Validate templates without building or signing")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not args.version:
        parser.error("--version is required")
    version_parts(args.version)
    if sys.platform not in ("win32", "darwin", "linux"):
        raise ValueError("Run packaging on Windows, macOS or Linux")
    binary = (args.binary or DESKTOP / "target/release" / ("koyori.exe" if sys.platform == "win32" else "koyori")).resolve()
    if not binary.is_file():
        raise ValueError(f"Release binary missing: {binary}; run cargo build --locked --release -p app first")
    try:
        arch = binary_arch(binary, sys.platform)
    except (KeyError, struct.error) as error:
        raise ValueError("Unsupported or malformed executable architecture") from error
    out = (args.out or DESKTOP / "target/packages" / f"{args.version}-{sys.platform}-{arch}").resolve()
    if out.exists():
        raise ValueError(f"Output directory already exists; choose a new --out: {out}")
    build = {"win32": windows, "darwin": macos, "linux": linux}[sys.platform]
    artifacts = build(binary, args.version, arch, out, args.unsigned)
    sums = out / "SHA256SUMS"
    hashes = []
    for path in artifacts:
        with path.open("rb") as artifact:
            hashes.append(f"{hashlib.file_digest(artifact, 'sha256').hexdigest()}  {path.name}\n")
    sums.write_text("".join(hashes), encoding="utf-8")
    print(f"{'UNSIGNED LOCAL TEST' if args.unsigned else 'Signed artifacts'}: {out}")
    print("No files were installed or published. Review the artifacts before distribution.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        sys.exit(1)
