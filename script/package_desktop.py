#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description='Build a Windows or Linux release archive.')
    parser.add_argument('--output-dir', type=Path, default=ROOT / '.build/release')
    args = parser.parse_args()
    system = platform.system().lower()
    if system not in ('windows', 'linux'):
        parser.error('Run this on Windows or Linux. On macOS use package_release.sh.')
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'peep'], cwd=ROOT, check=True)
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    name = f'Peep-{version}-{system}-{platform.machine().lower()}'
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    suffix = '.zip' if system == 'windows' else '.tar.gz'
    archive = output / (name + suffix)
    with tempfile.TemporaryDirectory(prefix='.package-', dir=output) as temporary:
        staging = Path(temporary)
        bundle = staging / name
        bundle.mkdir()
        executable = 'peep.exe' if system == 'windows' else 'peep'
        shutil.copy2(ROOT / 'target/release' / executable, bundle / executable)
        shutil.copy2(ROOT / 'LICENSE', bundle / 'LICENSE')
        if system == 'linux':
            shutil.copy2(ROOT / 'assets/dev.peep.app.desktop', bundle / 'dev.peep.app.desktop')
            shutil.copy2(ROOT / 'assets/icons/PeepIcon-128.png', bundle / 'dev.peep.app.png')
        shutil.copytree(ROOT / 'assets/notices', bundle / 'notices')
        package = shutil.make_archive(str(staging / 'release'), 'zip' if system == 'windows' else 'gztar',
                                      root_dir=staging, base_dir=name)
        os.link(package, archive)
    print(archive)


if __name__ == '__main__':
    main()
