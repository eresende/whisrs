# Moonshine ASR sidecar

This helper exposes Useful Sensors Moonshine as the local HTTP endpoint expected
by whisrs:

```text
POST http://127.0.0.1:8765/transcribe
```

Moonshine is a lightweight ASR family designed for fast local transcription.
The default model is `UsefulSensors/moonshine-base` for English. You can also
use `UsefulSensors/moonshine-tiny` for lower memory and faster startup.

## Requirements

- Python 3.11+
- `ffmpeg` available on `PATH`
- PyTorch for your CPU/GPU runtime

Moonshine is much smaller than VibeVoice-ASR. The English models are:

| Model | Parameters | Notes |
|---|---:|---|
| `UsefulSensors/moonshine-tiny` | 27M | Fastest, lowest memory |
| `UsefulSensors/moonshine-base` | 61M | Better accuracy, still lightweight |

## Install

Create an isolated Python environment:

```bash
cd contrib/asr-sidecars/moonshine
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

If the generic `torch` wheel is not appropriate for your GPU, install the
correct PyTorch build first, then run:

```bash
pip install -r requirements.txt
```

For AMD ROCm, verify PyTorch sees the GPU:

```bash
python3 - <<'PY'
import torch
print("torch:", torch.__version__)
print("cuda available:", torch.cuda.is_available())
print("device count:", torch.cuda.device_count())
if torch.cuda.is_available():
    print("device:", torch.cuda.get_device_name(0))
PY
```

## Run

```bash
python server.py --host 127.0.0.1 --port 8765
```

Useful options:

```bash
python server.py \
  --model UsefulSensors/moonshine-base \
  --device cuda:0 \
  --dtype float16
```

CPU mode:

```bash
python server.py --device cpu --dtype float32
```

If you see truncation or repetition on unusual inputs, override the generation
limit:

```bash
python server.py --max-length 128
```

## Configure whisrs

Set:

```toml
[general]
backend = "asr-sidecar"
language = "en"

[asr-sidecar]
url = "http://127.0.0.1:8765/transcribe"
model = "UsefulSensors/moonshine-base"
```

The same example is available in `config.toml.example`.

Then restart `whisrsd`.

## Test the sidecar

```bash
curl -F file=@/path/to/audio.wav \
  -F model=UsefulSensors/moonshine-base \
  -F language=en \
  http://127.0.0.1:8765/transcribe
```

Expected response:

```json
{ "text": "transcribed text" }
```
