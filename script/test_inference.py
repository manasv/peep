#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def fetch(url, output, size, digest):
    if output.exists() and output.stat().st_size == size:
        with output.open('rb') as source:
            if hashlib.file_digest(source, 'sha256').hexdigest() == digest:
                return
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix('.download')
    try:
        with urllib.request.urlopen(url, timeout=60) as source, temporary.open('wb') as target:
            while chunk := source.read(1024 * 1024):
                target.write(chunk)
        with temporary.open('rb') as source:
            valid = hashlib.file_digest(source, 'sha256').hexdigest() == digest
        if temporary.stat().st_size != size or not valid:
            raise RuntimeError(f'Verification failed: {output.name}')
        temporary.replace(output)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description='Download pinned public fixtures and test offline inference.')
    parser.add_argument('--backend', choices=['coreml', 'onnx'], default='onnx')
    args = parser.parse_args()
    manifest = json.loads((ROOT / f'crates/peep-runtime/assets/parakeet-{args.backend}.json').read_text())
    model = ROOT / '.build/models' / ('scriptrs' if args.backend == 'coreml' else 'parakeet-onnx')
    base = f"https://huggingface.co/{manifest['repository']}/resolve/{manifest['revision']}"
    for asset in manifest['assets']:
        print(f"Verifying {asset['path']}", flush=True)
        fetch(f"{base}/{asset['path']}", model / asset['path'], asset['bytes'], asset['sha256'])
    audio = ROOT / '.build/fixtures/parakeet-en.wav'
    fetch(
        'https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/'
        'resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/test_wavs/en.wav',
        audio, 184608, '148b936b43ce7c546a866e64da059f0458aee2d65e617f16e9d94f06e8d99ed6',
    )
    environment = dict(os.environ, PEEP_TEST_AUDIO_PATH=str(audio))
    variable = 'PEEP_TEST_MODEL_PATH' if args.backend == 'coreml' else 'PEEP_TEST_ONNX_MODEL_PATH'
    environment[variable] = str(model)
    package = 'peep-macos' if args.backend == 'coreml' else 'peep-desktop'
    subprocess.run(['cargo', 'test', '-p', package, '--locked', 'local_fixture_transcribes', '--', '--nocapture'],
                   cwd=ROOT, env=environment, check=True)


if __name__ == '__main__':
    main()
