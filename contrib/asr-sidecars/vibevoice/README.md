# VibeVoice-ASR sidecar

This helper exposes Microsoft VibeVoice-ASR as the local HTTP endpoint expected
by whisrs:

```text
POST http://127.0.0.1:8765/transcribe
```

It uses the Transformers-compatible model `microsoft/VibeVoice-ASR-HF`.

## Requirements

- NVIDIA GPU strongly recommended
- Python 3.11+
- `ffmpeg` available on `PATH`
- CUDA-compatible PyTorch installation for your system
- 16+ GB of free disk space for the model cache, plus temporary download space

Microsoft's upstream docs recommend an NVIDIA PyTorch container for the CUDA
environment. The model is designed for long-form ASR, supports over 50
languages, and can use customized context/hotwords.

## Hugging Face access

A Hugging Face account is not strictly required if the public model download
works anonymously. For a model this large, a token is still recommended because
unauthenticated requests can be slower or more rate-limited. A token is required
only if Hugging Face returns a gated-model/authentication error or asks you to
accept model terms.

Temporary token for one shell:

```bash
export HF_TOKEN=hf_...
python server.py --host 127.0.0.1 --port 8765
```

Persistent login:

```bash
huggingface-cli login
```

Model files are cached by Hugging Face, usually under
`~/.cache/huggingface/hub`, so later sidecar starts should not redownload the
full model.

## Install

Create an isolated Python environment:

```bash
cd contrib/asr-sidecars/vibevoice
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

If the generic `torch` wheel is not appropriate for your CUDA version, install
PyTorch from the official PyTorch index first, then run:

```bash
pip install -r requirements.txt
```

The sidecar uses `device_map=auto` by default, so `accelerate` is required by
Transformers for model placement.

Transformers loads uploaded audio through `librosa`, which is included in
`requirements.txt`. If you install dependencies manually, make sure both
`accelerate` and `librosa` are present.

## Run

```bash
python server.py --host 127.0.0.1 --port 8765
```

The first launch downloads the model into the Hugging Face cache.

Useful options:

```bash
python server.py \
  --model microsoft/VibeVoice-ASR-HF \
  --dtype float16 \
  --device-map auto
```

`float16` is the default because it works better across consumer GPUs. If you
are running on NVIDIA hardware with good bfloat16 support, `--dtype bfloat16`
is also worth testing.

### AMD ROCm notes

PyTorch exposes ROCm devices through the `torch.cuda` API. Verify the sidecar
environment sees the GPU before starting the server:

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

For RDNA2 cards such as the Radeon RX 6800 XT, use `float16`. If you see
`HIP error: invalid device function`, make sure the server is not running with
`--dtype bfloat16`:

```bash
python server.py --host 127.0.0.1 --port 8765 --dtype float16
```

If `torch.cuda.device_count()` reports more than one device, pin the sidecar to
the discrete GPU so `device_map=auto` does not place layers on an iGPU or other
ROCm-visible device:

```bash
HIP_VISIBLE_DEVICES=0 \
ROCR_VISIBLE_DEVICES=0 \
python server.py --host 127.0.0.1 --port 8765 --dtype float16
```

If ROCm still reports kernel/device issues on gfx1030 hardware, these
environment variables are common troubleshooting knobs:

```bash
HIP_VISIBLE_DEVICES=0 \
ROCR_VISIBLE_DEVICES=0 \
HSA_OVERRIDE_GFX_VERSION=10.3.0 \
HSA_ENABLE_SDMA=0 \
python server.py --host 127.0.0.1 --port 8765 --dtype float16
```

For ROCm memory aperture violations or out-of-memory failures, keep recordings
short and lower generation memory. The `--tokenizer-chunk-size` option is
best-effort; some Transformers/VibeVoice versions do not accept it, and the
sidecar will retry without it if rejected.

```bash
HIP_VISIBLE_DEVICES=0 \
ROCR_VISIBLE_DEVICES=0 \
python server.py \
  --host 127.0.0.1 \
  --port 8765 \
  --dtype float16 \
  --max-new-tokens 2048 \
  --tokenizer-chunk-size 64000
```

For lower-memory devices, try reducing the tokenizer chunk size:

```bash
python server.py --tokenizer-chunk-size 64000
```

## Configure whisrs

Set:

```toml
[general]
backend = "asr-sidecar"
language = "auto"

[asr-sidecar]
url = "http://127.0.0.1:8765/transcribe"
model = "microsoft/VibeVoice-ASR-HF"
```

The same example is available in `config.toml.example`.

Then restart `whisrsd`.

## Test the sidecar

```bash
curl -F file=@/path/to/audio.wav \
  -F model=microsoft/VibeVoice-ASR-HF \
  -F language=auto \
  -F hotwords="whisrs,Hyprland,VibeVoice" \
  http://127.0.0.1:8765/transcribe
```

Expected response:

```json
{ "text": "transcribed text" }
```
