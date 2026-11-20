"""Builds gglint's npm packages from the program built for each platform.

    python npm/build.py --version 0.5.0 --out DIR --wheels DIST
    python npm/build.py --version 0.5.0 --out DIR --binary linux-x64=target/release/gglint

`@gglint/cli` is a small Node script, `npm/gglint.js`, that runs the program from
`@gglint/cli-<platform>-<arch>`, an optional dependency npm installs only on the machine it
was built for. The release takes each program from its wheel, so PyPI and npm ship the same
files; `--binary` packs one built locally, for testing. The packages are written to DIR, one
folder each, and their folders are printed in the order to publish them: the platforms
first, so `@gglint/cli`'s dependencies exist when it's published.
"""

import argparse
import json
import re
import shutil
import sys
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCOPE = "@gglint"
# npm refuses `gglint` alone: too close to eslint, oxlint and tslint.
MAIN = f"{SCOPE}/cli"

# Node's `process.platform-process.arch`, and the wheel that holds the program for it. On
# Linux the musl build is static, so it runs on glibc and musl systems alike.
PLATFORMS = {
    "darwin-x64": re.compile(r"-macosx_\d+_\d+_x86_64\.whl$"),
    "darwin-arm64": re.compile(r"-macosx_\d+_\d+_arm64\.whl$"),
    "linux-x64": re.compile(r"-musllinux_\d+_\d+_x86_64\.whl$"),
    "linux-arm64": re.compile(r"-musllinux_\d+_\d+_aarch64\.whl$"),
    "win32-x64": re.compile(r"-win_amd64\.whl$"),
    "win32-arm64": re.compile(r"-win_arm64\.whl$"),
}

NAMES = {"darwin": "macOS", "linux": "Linux", "win32": "Windows"}


def program_name(platform: str) -> str:
    return "gglint.exe" if platform.startswith("win32") else "gglint"


def common(version: str) -> dict:
    """The fields every package shares, from the Python package's own details."""
    project = tomllib.loads((ROOT / "pyproject.toml").read_text())["project"]
    return {
        "version": version,
        "license": project["license"],
        "homepage": project["urls"]["Homepage"],
        "repository": {"type": "git", "url": "git+https://github.com/corridor/gglint.git"},
        "bugs": {"url": project["urls"]["Issues"]},
    }


def write_package(folder: Path, manifest: dict, files: dict[str, Path], readme: str | None = None):
    if folder.exists():
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    for relative, source in files.items():
        target = folder / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        if relative.startswith("bin/"):
            target.chmod(0o755)
    for license_file in ("LICENSE", "THIRD_PARTY_LICENSES"):
        shutil.copyfile(ROOT / license_file, folder / license_file)
    if readme is not None:
        (folder / "README.md").write_text(readme)
    (folder / "package.json").write_text(json.dumps(manifest, indent=2) + "\n")


def platform_package(out: Path, version: str, platform: str, binary: Path) -> Path:
    os_name, cpu = platform.split("-")
    name = f"{SCOPE}/cli-{platform}"
    manifest = {
        "name": name,
        **common(version),
        "description": f"The gglint program for {NAMES[os_name]} on {cpu}; install `{MAIN}` instead",
        "os": [os_name],
        "cpu": [cpu],
        "files": ["bin/", "LICENSE", "THIRD_PARTY_LICENSES"],
        # Yarn's Plug'n'Play keeps packages zipped unless they ask; a program has to be a file.
        "preferUnplugged": True,
    }
    readme = f"# {name}\n\nThe gglint program for {NAMES[os_name]} on {cpu}. Install [`{MAIN}`](https://www.npmjs.com/package/{MAIN}), which installs this on the machines it's for.\n"
    folder = out / f"cli-{platform}"
    write_package(folder, manifest, {f"bin/{program_name(platform)}": binary}, readme)
    return folder


def main_package(out: Path, version: str) -> Path:
    project = tomllib.loads((ROOT / "pyproject.toml").read_text())["project"]
    manifest = {
        "name": MAIN,
        **common(version),
        "description": project["description"],
        "keywords": project["keywords"],
        "bin": {"gglint": "bin/gglint.js"},
        "files": ["bin/", "README.md", "LICENSE", "THIRD_PARTY_LICENSES"],
        "engines": {"node": ">=14"},
        "optionalDependencies": {f"{SCOPE}/cli-{platform}": version for platform in PLATFORMS},
    }
    folder = out / "cli"
    write_package(
        folder,
        manifest,
        {"bin/gglint.js": ROOT / "npm" / "gglint.js", "README.md": ROOT / "README.md"},
    )
    return folder


def programs_from_wheels(wheels: Path, version: str, unpacked: Path) -> dict[str, Path]:
    """Each platform's program, taken out of its wheel, which must be this version."""
    found = {}
    for wheel in sorted(wheels.glob("*.whl")):
        platform = next((p for p, pattern in PLATFORMS.items() if pattern.search(wheel.name)), None)
        if platform is None:
            continue
        if not wheel.name.startswith(f"gglint-{version}-"):
            sys.exit(f"error: {wheel.name} isn't version {version}")
        member = f"gglint-{version}.data/scripts/{program_name(platform)}"
        target = unpacked / platform / program_name(platform)
        target.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(wheel) as archive, archive.open(member) as source:
            target.write_bytes(source.read())
        found[platform] = target
    missing = sorted(set(PLATFORMS) - set(found))
    if missing:
        sys.exit(f"error: no wheel in {wheels} for {', '.join(missing)}")
    return found


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", required=True, type=Path)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--wheels", type=Path, help="a folder of the release's wheels")
    source.add_argument("--binary", action="append", metavar="PLATFORM=PATH", help="a program built locally")
    args = parser.parse_args()

    if args.wheels:
        programs = programs_from_wheels(args.wheels, args.version, args.out / ".programs")
    else:
        programs = {}
        for pair in args.binary:
            platform, _, path = pair.partition("=")
            if platform not in PLATFORMS or not path:
                sys.exit(f"error: {pair!r} isn't PLATFORM=PATH, with PLATFORM one of {', '.join(PLATFORMS)}")
            programs[platform] = Path(path)

    folders = [platform_package(args.out, args.version, p, programs[p]) for p in PLATFORMS if p in programs]
    folders.append(main_package(args.out, args.version))
    for folder in folders:
        print(folder)


if __name__ == "__main__":
    main()
