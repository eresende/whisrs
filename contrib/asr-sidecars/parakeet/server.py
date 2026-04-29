#!/usr/bin/env python3
"""HTTP sidecar for NVIDIA Parakeet ASR.

The whisrs `asr-sidecar` backend posts WAV audio to `/transcribe` as multipart
form data. This sidecar loads a NeMo-compatible Parakeet model once at startup
and returns a plain transcript as JSON.
"""

from __future__ import annotations

import argparse
import os
import tempfile
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

import torch
from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from nemo.collections import asr as nemo_asr
from omegaconf import OmegaConf


DEFAULT_MODEL = "nvidia/parakeet-tdt-0.6b-v3"


def _first_text(value: Any) -> str:
    """Normalize NeMo transcription outputs across versions."""
    if isinstance(value, str):
        return value.strip()
    if hasattr(value, "text"):
        return str(value.text).strip()
    if isinstance(value, (list, tuple)) and value:
        return _first_text(value[0])
    return str(value).strip()


class ParakeetSidecar:
    def __init__(
        self,
        model_id: str,
        device: str,
        batch_size: int,
        use_cuda_graph_decoder: bool,
    ) -> None:
        self.model_id = model_id
        self.device = torch.device(device)
        self.batch_size = batch_size

        cfg = nemo_asr.models.ASRModel.from_pretrained(model_id, return_config=True)
        OmegaConf.update(
            cfg,
            "decoding.greedy.use_cuda_graph_decoder",
            use_cuda_graph_decoder,
            merge=True,
        )
        with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as tmp:
            cfg_path = Path(tmp.name)
            OmegaConf.save(cfg, tmp)

        try:
            self.model = nemo_asr.models.ASRModel.from_pretrained(
                model_name=model_id,
                override_config_path=str(cfg_path),
                map_location=self.device,
            )
        finally:
            cfg_path.unlink(missing_ok=True)
        self.model.to(self.device)
        self.model.eval()

    @torch.inference_mode()
    def transcribe(self, audio_path: Path) -> str:
        output = self.model.transcribe(
            [str(audio_path)],
            batch_size=self.batch_size,
        )
        return _first_text(output)


def create_app(sidecar: ParakeetSidecar) -> FastAPI:
    @asynccontextmanager
    async def lifespan(app: FastAPI):
        app.state.sidecar = sidecar
        yield

    app = FastAPI(title="whisrs Parakeet ASR sidecar", lifespan=lifespan)

    @app.get("/health")
    async def health() -> dict[str, str]:
        return {"status": "ok", "model": app.state.sidecar.model_id}

    @app.post("/transcribe")
    async def transcribe(
        file: UploadFile = File(...),
        model: str = Form(DEFAULT_MODEL),
        language: str | None = Form(None),
        hotwords: str | None = Form(None),
        prompt: str | None = Form(None),
    ) -> dict[str, str]:
        if model != app.state.sidecar.model_id:
            raise HTTPException(
                status_code=400,
                detail=(
                    f"sidecar loaded {app.state.sidecar.model_id}, "
                    f"but request asked for {model}"
                ),
            )
        # Parakeet v3 auto-detects supported languages. The generic whisrs
        # sidecar contract includes these fields, but basic NeMo Parakeet
        # inference does not consume them directly.
        _ = language
        _ = hotwords or prompt

        suffix = Path(file.filename or "audio.wav").suffix or ".wav"
        with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as tmp:
            tmp_path = Path(tmp.name)
            tmp.write(await file.read())

        try:
            text = app.state.sidecar.transcribe(tmp_path)
        finally:
            tmp_path.unlink(missing_ok=True)

        return {"text": text}

    return app


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default=os.getenv("PARAKEET_HOST", "127.0.0.1"))
    parser.add_argument(
        "--port",
        type=int,
        default=int(os.getenv("PARAKEET_PORT", "8765")),
    )
    parser.add_argument("--model", default=os.getenv("PARAKEET_MODEL", DEFAULT_MODEL))
    parser.add_argument(
        "--device",
        default=os.getenv(
            "PARAKEET_DEVICE",
            "cuda:0" if torch.cuda.is_available() else "cpu",
        ),
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=int(os.getenv("PARAKEET_BATCH_SIZE", "1")),
        help="Batch size passed to NeMo transcribe(). Keep at 1 for dictation.",
    )
    parser.add_argument(
        "--use-cuda-graph-decoder",
        action="store_true",
        default=os.getenv("PARAKEET_USE_CUDA_GRAPH_DECODER", "0") == "1",
        help=(
            "Enable NeMo CUDA graph decoding. Faster on NVIDIA, but disable it "
            "on ROCm/AMD because cuda-python expects libcuda.so.1."
        ),
    )
    args = parser.parse_args()

    import uvicorn

    sidecar = ParakeetSidecar(
        model_id=args.model,
        device=args.device,
        batch_size=args.batch_size,
        use_cuda_graph_decoder=args.use_cuda_graph_decoder,
    )
    uvicorn.run(create_app(sidecar), host=args.host, port=args.port)


if __name__ == "__main__":
    main()
