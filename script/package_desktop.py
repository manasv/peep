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


def windows_architecture(executable):
    with executable.open('rb') as image:
        if image.read(2) != b'MZ':
            raise RuntimeError(f'Invalid Windows executable: {executable}')
        image.seek(0x3C)
        offset = int.from_bytes(image.read(4), 'little')
        image.seek(offset)
        if image.read(4) != b'PE\0\0':
            raise RuntimeError(f'Invalid PE header: {executable}')
        machine = int.from_bytes(image.read(2), 'little')
    architectures = {0x8664: 'amd64', 0xAA64: 'arm64'}
    if machine not in architectures:
        raise RuntimeError(f'Unsupported Windows executable architecture: 0x{machine:04X}')
    return architectures[machine]


def main():
    parser = argparse.ArgumentParser(description='Build a Windows or Linux release archive.')
    parser.add_argument('--output-dir', type=Path, default=ROOT / '.build/release')
    args = parser.parse_args()
    system = platform.system().lower()
    if system not in ('windows', 'linux'):
        parser.error('Run this on Windows or Linux. On macOS use package_release.sh.')
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'peep'], cwd=ROOT, check=True)
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    if not target.is_absolute():
        target = ROOT / target
    release = target / 'release'
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    executable = 'peep.exe' if system == 'windows' else 'peep'
    architecture = windows_architecture(release / executable) if system == 'windows' else platform.machine().lower()
    name = f'Peep-{version}-{system}-{architecture}'
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    suffix = '.zip' if system == 'windows' else '.tar.gz'
    archive = output / (name + suffix)
    with tempfile.TemporaryDirectory(prefix='.package-', dir=output) as temporary:
        staging = Path(temporary)
        bundle = staging / name
        bundle.mkdir()
        shutil.copy2(release / executable, bundle / executable)
        shutil.copy2(ROOT / 'LICENSE', bundle / 'LICENSE')
        if system == 'linux':
            for library in ('libsherpa-onnx-c-api.so', 'libonnxruntime.so'):
                shutil.copy2(release / library, bundle / library)
            environment = {key: value for key, value in os.environ.items() if key != 'LD_LIBRARY_PATH'}
            check = subprocess.run(['ldd', '-r', str(bundle / executable)],
                                   env=environment, capture_output=True, text=True)
            diagnostics = check.stdout + check.stderr
            if check.returncode or 'not found' in diagnostics or 'undefined symbol' in diagnostics:
                raise RuntimeError(f'Packaged Linux libraries could not be resolved:\n{diagnostics}')
            shutil.copy2(ROOT / 'assets/dev.peep.app.desktop', bundle / 'dev.peep.app.desktop')
            shutil.copy2(ROOT / 'assets/icons/PeepIcon-128.png', bundle / 'dev.peep.app.png')
        shutil.copytree(ROOT / 'assets/notices', bundle / 'notices')
        package = shutil.make_archive(str(staging / 'release'), 'zip' if system == 'windows' else 'gztar',
                                      root_dir=staging, base_dir=name)
        os.replace(package, archive)
    print(archive)


if __name__ == '__main__':
    main()
