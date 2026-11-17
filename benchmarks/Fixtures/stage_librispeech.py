#!/usr/bin/env python3
"""Stage the 27 published LibriSpeech clips used for the release benchmark."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile
from urllib.parse import urlencode, urlsplit


HOST = "datasets-server.huggingface.co"
DATASET = "openslr/librispeech_asr"
REVISION = "71cacbfb7e2354c4226d01e70d77d5fca3d04ba1"
ROWS = (
    0, 100, 200, 300, 400, 500, 601, 700, 800, 900, 1000, 1100, 1200, 1300,
    1400, 1500, 1600, 1700, 1800, 1901, 2001, 2100, 2200, 2300, 2400, 2500, 2600,
)


def fetch(url, destination):
    parts = urlsplit(url)
    if parts.scheme != "https" or parts.netloc != HOST:
        raise ValueError("Unexpected LibriSpeech download URL.")
    subprocess.run(
        ["/usr/bin/curl", "--fail", "--silent", "--show-error", "--location",
         "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "10",
         "--max-time", "45", "--retry", "1", "--output", str(destination), url],
        check=True,
    )


def stage(index, directory):
    query = urlencode(dict(dataset=DATASET, config="all", split="test.clean", offset=index, length=1))
    with tempfile.TemporaryDirectory(dir=directory) as scratch:
        scratch = Path(scratch)
        metadata = scratch / "row.json"
        fetch(f"https://{HOST}/rows?{query}", metadata)
        entries = json.loads(metadata.read_text())["rows"]
        if len(entries) != 1 or entries[0]["row_idx"] != index:
            raise ValueError(f"Unexpected dataset response for row {index}.")
        entry = entries[0]
        row = entry["row"]
        clip = row["id"]
        if not clip.replace("-", "").isascii() or not clip.replace("-", "").isdigit():
            raise ValueError(f"Invalid clip ID in row {index}.")
        source = next(asset["src"] for asset in row["audio"] if asset["type"] == "audio/flac")
        suffix = f"/{DATASET}/--/{REVISION}/--/all/test.clean/{index}/audio/audio.flac"
        if urlsplit(source).path not in {"/cached-assets" + suffix, "/assets" + suffix}:
            raise ValueError(f"The dataset revision changed for row {index}.")
        if "text" in entry.get("truncated_cells", []):
            raise ValueError(f"The dataset revision or reference changed for row {index}.")
        flac = scratch / "source.flac"
        wav = directory / f"{clip}.wav"
        fetch(source, flac)
        subprocess.run(
            ["/usr/bin/afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", str(flac), str(wav)],
            check=True,
        )
        duration = wav_duration(wav)
        return {
            "id": clip, "path": wav.name, "category": "LibriSpeech test.clean", "reference": row["text"],
            "speaker": row["speaker_id"], "chapter": row["chapter_id"], "row_index": index,
            "duration_seconds": duration, "source_flac_sha256": hashlib.sha256(flac.read_bytes()).hexdigest(),
            "wav_sha256": hashlib.sha256(wav.read_bytes()).hexdigest(),
        }


def wav_duration(path):
    with path.open("rb") as audio:
        riff = audio.read(12)
        if len(riff) != 12 or riff[:4] != b"RIFF" or riff[8:] != b"WAVE":
            raise ValueError(f"Expected a WAV file: {path.name}")
        while header := audio.read(8):
            name, size = struct.unpack("<4sI", header)
            if name == b"fmt " and 16 <= size <= 64:
                fields = audio.read(size)
                encoding, channels, rate, _, frame_size, bits = struct.unpack_from("<HHIIHH", fields)
                pcm = encoding == 1 or (encoding == 65534 and fields[24:40] == bytes.fromhex(
                    "0100000000001000800000aa00389b71"
                ))
                if not pcm or (channels, rate, frame_size, bits) != (1, 16_000, 2, 16):
                    break
                if size % 2:
                    audio.seek(1, 1)
                for chunk in iter(lambda: audio.read(8), b""):
                    kind, length = struct.unpack("<4sI", chunk)
                    if kind == b"data":
                        return length / frame_size / rate
                    audio.seek(length + length % 2, 1)
                break
            audio.seek(size + size % 2, 1)
    raise ValueError(f"Expected 16 kHz mono PCM in {path.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="A new or empty output directory")
    directory = parser.parse_args().output.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    if any(directory.iterdir()):
        parser.error(f"Use a new or empty output directory: {directory}")
    with ThreadPoolExecutor(max_workers=6) as pool:
        items = list(pool.map(lambda index: stage(index, directory), ROWS))
    manifest = directory / "manifest.json"
    manifest.write_text(json.dumps(items, indent=2) + "\n")
    print(manifest)


if __name__ == "__main__":
    main()
