#!/usr/bin/env python3
"""Write a synthetic English dictation smoke corpus using installed Mac voices."""

import argparse
import array
import json
import math
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import wave


SAMPLE_RATE = 16_000
CLIPS = (
    (
        "message",
        "clean",
        "Samantha",
        172,
        "Please send me the draft before the team call. I can review it this afternoon.",
    ),
    (
        "negation",
        "clean",
        "Daniel",
        165,
        "I didn't ask you to cancel the meeting. Please move it to Thursday instead.",
    ),
    (
        "question",
        "accent",
        "Karen",
        175,
        "Could you check whether the parcel arrived before we left the office?",
    ),
    (
        "directions",
        "accent",
        "Moira",
        168,
        "When you reach the station, turn left and meet me outside the bookshop.",
    ),
    (
        "people",
        "names",
        "Samantha",
        158,
        "Please invite Nadia Patel and Marcus Nguyen to the planning meeting.",
    ),
    (
        "places",
        "names",
        "Daniel",
        165,
        "The train leaves Edinburgh in the morning and arrives in Newcastle before lunch.",
    ),
    (
        "quantities",
        "numbers",
        "Karen",
        170,
        "We need fifteen chairs, fifty plates, and two hundred napkins.",
    ),
    (
        "decimal",
        "numbers",
        "Moira",
        157,
        "The package weighed two point five kilograms, and the scale was off by zero point one.",
    ),
    (
        "swift",
        "technical",
        "Samantha",
        173,
        "The Swift compiler reports a memory allocation error when we parse the JSON response.",
    ),
    (
        "git",
        "technical",
        "Daniel",
        173,
        "Open the terminal, run git status, and check whether the checkout has any changes.",
    ),
    (
        "message_noise",
        "noise",
        "Samantha",
        172,
        "Please send me the draft before the team call. I can review it this afternoon.",
    ),
    ("silence", "silence", None, 0, ""),
)

WRITTEN_NUMBERS = {
    "quantities": "We need 15 chairs, 50 plates, and 200 napkins.",
    "decimal": "The package weighed 2.5 kilograms, and the scale was off by 0.1.",
}


def read_samples(path):
    with wave.open(str(path), "rb") as audio:
        if (audio.getnchannels(), audio.getsampwidth(), audio.getframerate()) != (
            1,
            2,
            SAMPLE_RATE,
        ):
            raise ValueError(f"{path.name} is not 16 kHz mono PCM16")
        samples = array.array("h", audio.readframes(audio.getnframes()))
    if sys.byteorder == "big":
        samples.byteswap()
    return samples


def write_samples(path, samples):
    if sys.byteorder == "big":
        samples.byteswap()
    with wave.open(str(path), "wb") as audio:
        audio.setparams((1, 2, SAMPLE_RATE, 0, "NONE", "not compressed"))
        audio.writeframes(samples.tobytes())


def synthesize(path, voice, rate, reference):
    result = subprocess.run(
        [
            "/usr/bin/say",
            "-v",
            voice,
            "-r",
            str(rate),
            "-o",
            str(path),
            "--data-format=LEI16@16000",
            reference,
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        raise ValueError(f"Could not synthesize with {voice}: {result.stderr.strip()}")
    samples = read_samples(path)
    if not any(samples):
        raise ValueError(
            f"{voice} produced no audible samples. Check that the voice is installed "
            "and run the generator from a normal macOS Terminal session."
        )
    return samples


def add_noise(samples):
    generator = random.Random(2718)
    rms = math.sqrt(sum(sample * sample for sample in samples) / len(samples))
    noise_scale = rms * 10 ** (-12 / 20) * math.sqrt(3)
    for index, sample in enumerate(samples):
        noisy = round(sample + generator.uniform(-noise_scale, noise_scale))
        samples[index] = max(-32768, min(32767, noisy))


def generate(output):
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ValueError(f"Use a new or empty output directory: {output}")

    with tempfile.TemporaryDirectory(dir=output) as stage_name:
        stage = Path(stage_name)
        raw = stage / "raw.wav"
        manifest = []
        for clip_id, category, voice, rate, reference in CLIPS:
            filename = f"{clip_id}.wav"
            if voice:
                speech = synthesize(raw, voice, rate, reference)
                samples = array.array("h", [0]) * (SAMPLE_RATE // 4)
                samples.extend(speech)
                samples.extend(array.array("h", [0]) * (SAMPLE_RATE // 4))
                if category == "noise":
                    add_noise(samples)
            else:
                samples = array.array("h", [0]) * (SAMPLE_RATE * 4)
            write_samples(stage / filename, samples)
            manifest.append(
                {
                    "id": clip_id,
                    "path": filename,
                    "category": category,
                    "reference": reference,
                }
            )
            if clip_id in WRITTEN_NUMBERS:
                manifest[-1]["alternatives"] = [WRITTEN_NUMBERS[clip_id]]
        (stage / "manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        for clip in manifest:
            (stage / clip["path"]).replace(output / clip["path"])
        (stage / "manifest.json").replace(output / "manifest.json")

    print(output / "manifest.json")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="A new or empty output directory")
    args = parser.parse_args()
    try:
        generate(args.output.resolve())
    except (OSError, ValueError, EOFError, wave.Error, subprocess.TimeoutExpired) as exc:
        parser.exit(1, f"{exc}\n")


if __name__ == "__main__":
    main()
