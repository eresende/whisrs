# Parakeet ASR sidecar

This helper exposes NVIDIA Parakeet as the local HTTP endpoint expected by
whisrs:

```text
POST http://127.0.0.1:8765/transcribe
```

The default model is `nvidia/parakeet-tdt-0.6b-v3`, a NeMo ASR model with
automatic punctuation, capitalization, and multilingual support for European
languages.

## Requirements

- Python 3.11+
- `ffmpeg` available on `PATH`
- PyTorch for your CPU/GPU runtime
- NVIDIA NeMo ASR
- A GPU is recommended for the default 0.6B model

## Install

Create an isolated Python environment:

```bash
cd contrib/asr-sidecars/parakeet
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
  --model nvidia/parakeet-tdt-0.6b-v3 \
  --device cuda:0 \
  --batch-size 1
```

CPU mode is available but may be slow for the default model:

```bash
python server.py --device cpu
```

### AMD ROCm notes

PyTorch exposes ROCm devices through the `torch.cuda` API, so `--device cuda:0`
is still the right spelling when `torch.cuda.is_available()` is true. NeMo's
CUDA graph decoder expects NVIDIA's CUDA driver library (`libcuda.so.1`),
though, so this sidecar disables CUDA graph decoding by default.

For ROCm, start with:

```bash
HIP_VISIBLE_DEVICES=0 \
ROCR_VISIBLE_DEVICES=0 \
python server.py \
  --host 127.0.0.1 \
  --port 8765 \
  --model nvidia/parakeet-tdt-0.6b-v3 \
  --device cuda:0 \
  --batch-size 1
```

On NVIDIA, you can opt back into NeMo's CUDA graph decoder:

```bash
python server.py --device cuda:0 --use-cuda-graph-decoder
```

## Configure whisrs

Set:

```toml
[general]
backend = "asr-sidecar"
language = "auto"

[asr-sidecar]
url = "http://127.0.0.1:8765/transcribe"
model = "nvidia/parakeet-tdt-0.6b-v3"
```

The same example is available in `config.toml.example`.

Then restart `whisrsd`.

## Test the sidecar

```bash
curl -F file=@/path/to/audio.wav \
  -F model=nvidia/parakeet-tdt-0.6b-v3 \
  -F language=auto \
  http://127.0.0.1:8765/transcribe
```

Expected response:

```json
{ "text": "transcribed text" }
```
